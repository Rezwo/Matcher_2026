#![allow(dead_code)]
#![allow(non_snake_case)]

mod Types;
mod Configurations;
mod Matchmaker;

use axum::{
    extract::{Path, State as AxumState, Request},
    routing::{get, post, delete},
    Json as AxumJson, Router,
    http::{StatusCode, HeaderMap},
    middleware::{self, Next},
    response::{Response, IntoResponse},
};
use bb8::Pool;
use bb8_redis::RedisConnectionManager;
use chrono::Utc;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::time::{interval, Duration, sleep};
use tokio::sync::Notify;
use uuid::Uuid;
use serde_json::json;
use tracing::{info, warn, error};
use subtle::ConstantTimeEq;

use crate::Types::{
    MatchmakingTicket, PartyMember, MatchmakingConfiguration,
    SubmitTicketRequest, HealthResponse, ErrorResponse
};
use crate::Configurations::GetStandardConfiguration;
use crate::Matchmaker::Matchmaker as MatchmakingLogic;

const REDIS_TICKET_KEY: &str = "MATCHMAKING_QUEUES";

struct AppState {
    pub RedisPool: Pool<RedisConnectionManager>,
    pub HttpClient: reqwest::Client,
    pub MatchmakerInstance: MatchmakingLogic,
    pub ShuttingDown: AtomicBool,
    pub ShutdownNotify: Notify,
}

fn ConstantTimeCompare(A: &str, B: &str) -> bool {
    let ABytes = A.as_bytes();
    let BBytes = B.as_bytes();

    if ABytes.len() != BBytes.len() {
        return false;
    }

    ABytes.ct_eq(BBytes).into()
}

async fn AuthMiddleware(
    AxumState(Data): AxumState<Arc<AppState>>,
    Headers: HeaderMap,
    Req: Request,
    Next: Next,
) -> Result<Response, StatusCode> {
    let AuthHeader: &str = Headers
        .get("X-Api-Key")
        .and_then(|Value| Value.to_str().ok())
        .unwrap_or("");

    if !ConstantTimeCompare(AuthHeader, &Data.MatchmakerInstance.Configuration.ServerAuthSecret) {
        warn!("Unauthorized access attempt detected.");
        return Err(StatusCode::UNAUTHORIZED);
    }

    Ok(Next.run(Req).await)
}

async fn ShutdownSignal(State: Arc<AppState>) {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to install CTRL+C signal handler");

    info!("Shutdown signal received. Draining queue...");
    State.ShuttingDown.store(true, Ordering::SeqCst);
    State.ShutdownNotify.notified().await;
    info!("Queue drained. Closing gracefully...");
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    info!("Initializing Production Matchmaker Server...");

    let Config: MatchmakingConfiguration = GetStandardConfiguration();
    let RedisUrl: String = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

    let Manager: RedisConnectionManager = RedisConnectionManager::new(RedisUrl.clone())
        .expect("Invalid Redis URL");

    let RedisPool: Pool<RedisConnectionManager> = Pool::builder()
        .max_size(16)
        .min_idle(Some(4))
        .build(Manager)
        .await
        .expect("Failed to create Redis connection pool");

    match RedisPool.get().await {
        Ok(_) => info!("Successfully connected to Redis at {}", RedisUrl),
        Err(Error) => {
            error!("CRITICAL: Could not connect to Redis. Ensure your Docker container is running.");
            error!("Error Details: {}", Error);
            return;
        }
    }

    let HttpClient: reqwest::Client = reqwest::Client::builder()
        .pool_max_idle_per_host(10)
        .timeout(Duration::from_secs(30))
        .build()
        .expect("Failed to create HTTP client");

    let SharedState: Arc<AppState> = Arc::new(AppState {
        RedisPool,
        HttpClient,
        MatchmakerInstance: MatchmakingLogic::new(Config),
        ShuttingDown: AtomicBool::new(false),
        ShutdownNotify: Notify::new(),
    });

    let LoopState: Arc<AppState> = SharedState.clone();
    tokio::spawn(async move {
        let TickRate: u64 = LoopState.MatchmakerInstance.Configuration.MatchmakerTickRateSeconds;
        let mut Interval: tokio::time::Interval = interval(Duration::from_secs(TickRate));

        loop {
            Interval.tick().await;

            if LoopState.ShuttingDown.load(Ordering::SeqCst) {
                LoopState.ShutdownNotify.notify_one();
                break;
            }

            let mut Connection = match LoopState.RedisPool.get().await {
                Ok(Conn) => Conn,
                Err(Error) => {
                    error!("Redis Connection Error: {}", Error);
                    continue;
                }
            };

            let RawTickets: Vec<String> = redis::cmd("HVALS")
                .arg(REDIS_TICKET_KEY)
                .query_async(&mut *Connection)
                .await
                .unwrap_or_default();

            let mut Tickets: Vec<MatchmakingTicket> = RawTickets
                .iter()
                .filter_map(|Serialized| serde_json::from_str::<MatchmakingTicket>(Serialized).ok())
                .collect();

            if Tickets.is_empty() {
                continue;
            }

            LoopState.MatchmakerInstance.ExpandTickets(&mut Tickets);
            let Matches: Vec<Vec<MatchmakingTicket>> = LoopState.MatchmakerInstance.FindMatches(&Tickets);
            let MatchedIds: HashSet<Uuid> = LoopState.MatchmakerInstance.GetMatchedTicketIds(&Matches);

            for FoundMatch in Matches {
                let MatchId: Uuid = Uuid::new_v4();
                let Members: Vec<PartyMember> = FoundMatch.iter().flat_map(|Ticket| Ticket.Members.clone()).collect();
                let MatchedTicketIds: Vec<String> = FoundMatch.iter().map(|Ticket| Ticket.TicketId.to_string()).collect();

                let InternalState: Arc<AppState> = LoopState.clone();
                tokio::spawn(async move {
                    match SendMatchToRobloxWithRetry(MatchId, Members, InternalState.clone()).await {
                        Ok(_) => {
                            if let Ok(mut Con) = InternalState.RedisPool.get().await {
                                let _: Result<(), _> = redis::cmd("HDEL")
                                    .arg(REDIS_TICKET_KEY)
                                    .arg(MatchedTicketIds)
                                    .query_async(&mut *Con)
                                    .await;
                            }
                        }
                        Err(Error) => {
                            error!("FATAL: Match notification failed: {}", Error);
                        }
                    }
                });
            }

            let UnmatchedTickets: Vec<&MatchmakingTicket> = Tickets
                .iter()
                .filter(|T| !MatchedIds.contains(&T.TicketId))
                .collect();

            if !UnmatchedTickets.is_empty() {
                let mut Pipeline = redis::pipe();
                for Ticket in UnmatchedTickets {
                    if let Ok(SerializedTicket) = serde_json::to_string(&Ticket) {
                        Pipeline.cmd("HSET")
                            .arg(REDIS_TICKET_KEY)
                            .arg(Ticket.TicketId.to_string())
                            .arg(SerializedTicket)
                            .ignore();
                    }
                }
                let _: Result<(), _> = Pipeline.query_async(&mut *Connection).await;
            }
        }
    });

    let ProtectedRoutes: Router<Arc<AppState>> = Router::new()
        .route("/submit", post(SubmitTicket))
        .route("/ticket/{TicketId}", delete(CancelTicket))
        .layer(middleware::from_fn_with_state(SharedState.clone(), AuthMiddleware));

    let App: Router = Router::new()
        .route("/health", get(HealthCheck))
        .merge(ProtectedRoutes)
        .with_state(SharedState.clone());

    let Addr: &str = "0.0.0.0:3000";
    let Listener: tokio::net::TcpListener = tokio::net::TcpListener::bind(Addr).await.unwrap();
    info!("Server Online. Listening on http://{}", Addr);

    let ShutdownState = SharedState.clone();
    axum::serve(Listener, App)
        .with_graceful_shutdown(ShutdownSignal(ShutdownState))
        .await
        .unwrap();
}

async fn HealthCheck(
    AxumState(Data): AxumState<Arc<AppState>>,
) -> impl IntoResponse {
    let RedisConnected: bool = Data.RedisPool.get().await.is_ok();

    let Response = HealthResponse {
        Status: if RedisConnected { "healthy".to_string() } else { "degraded".to_string() },
        RedisConnected,
    };

    let StatusCode = if RedisConnected { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };
    (StatusCode, AxumJson(Response))
}

async fn SubmitTicket(
    AxumState(Data): AxumState<Arc<AppState>>,
    AxumJson(Request): AxumJson<SubmitTicketRequest>,
) -> Result<AxumJson<Uuid>, (StatusCode, AxumJson<ErrorResponse>)> {
    let Config = &Data.MatchmakerInstance.Configuration;

    if Request.Members.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            AxumJson(ErrorResponse { Error: "Members array cannot be empty".to_string() }),
        ));
    }

    if Request.Members.len() > Config.MaximumPartySize as usize {
        warn!("Rejected ticket: Party size {} exceeds maximum", Request.Members.len());
        return Err((
            StatusCode::BAD_REQUEST,
            AxumJson(ErrorResponse {
                Error: format!("Party size exceeds maximum of {}", Config.MaximumPartySize)
            }),
        ));
    }

    for Member in &Request.Members {
        if Member.MatchmakingRating.is_nan() || Member.MatchmakingRating.is_infinite() {
            return Err((
                StatusCode::BAD_REQUEST,
                AxumJson(ErrorResponse { Error: "Invalid rating: NaN or Infinity not allowed".to_string() }),
            ));
        }
        if Member.MatchmakingRating < Config.MinimumRating || Member.MatchmakingRating > Config.MaximumRating {
            return Err((
                StatusCode::BAD_REQUEST,
                AxumJson(ErrorResponse {
                    Error: format!("Rating must be between {} and {}", Config.MinimumRating, Config.MaximumRating)
                }),
            ));
        }
    }

    let PreferredRegion: String = Request.PreferredRegion
        .filter(|R| Config.ValidRegions.contains(R))
        .unwrap_or_else(|| Config.DefaultRegion.clone());

    let TicketId: Uuid = Uuid::new_v4();
    let AverageRating: f64 = Request.Members.iter().map(|M| M.MatchmakingRating).sum::<f64>() / Request.Members.len() as f64;
    let InitialRange: f64 = Config.InitialMatchmakingRatingRange;

    let mut AllowedRegions: HashSet<String> = HashSet::new();
    AllowedRegions.insert(PreferredRegion.clone());

    let NewTicket: MatchmakingTicket = MatchmakingTicket {
        TicketId,
        Members: Request.Members,
        AverageMatchmakingRating: AverageRating,
        PreferredRegion: PreferredRegion.clone(),
        AllowedRegions,
        SubmittedTimestamp: Utc::now().timestamp(),
        SearchExpansionLevel: 0,
        MinimumMatchmakingRating: AverageRating - InitialRange,
        MaximumMatchmakingRating: AverageRating + InitialRange,
    };

    let mut Connection = Data.RedisPool.get().await.map_err(|_| (
        StatusCode::INTERNAL_SERVER_ERROR,
        AxumJson(ErrorResponse { Error: "Failed to connect to Redis".to_string() }),
    ))?;

    let SerializedTicket: String = serde_json::to_string(&NewTicket).map_err(|_| (
        StatusCode::INTERNAL_SERVER_ERROR,
        AxumJson(ErrorResponse { Error: "Failed to serialize ticket".to_string() }),
    ))?;

    let _: () = redis::cmd("HSET")
        .arg(REDIS_TICKET_KEY)
        .arg(TicketId.to_string())
        .arg(SerializedTicket)
        .query_async(&mut *Connection)
        .await
        .map_err(|_| (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(ErrorResponse { Error: "Failed to store ticket in Redis".to_string() }),
        ))?;

    info!("+ Party Ticket Queued: {} (Size: {}, Region: {})", TicketId, NewTicket.Members.len(), PreferredRegion);
    Ok(AxumJson(TicketId))
}

async fn CancelTicket(
    AxumState(Data): AxumState<Arc<AppState>>,
    Path(TicketId): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, AxumJson<ErrorResponse>)> {
    let mut Connection = Data.RedisPool.get().await.map_err(|_| (
        StatusCode::INTERNAL_SERVER_ERROR,
        AxumJson(ErrorResponse { Error: "Failed to connect to Redis".to_string() }),
    ))?;

    let Deleted: i32 = redis::cmd("HDEL")
        .arg(REDIS_TICKET_KEY)
        .arg(TicketId.to_string())
        .query_async(&mut *Connection)
        .await
        .map_err(|_| (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(ErrorResponse { Error: "Failed to delete ticket from Redis".to_string() }),
        ))?;

    if Deleted > 0 {
        info!("- Ticket Cancelled: {}", TicketId);
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((
            StatusCode::NOT_FOUND,
            AxumJson(ErrorResponse { Error: "Ticket not found".to_string() }),
        ))
    }
}

async fn SendMatchToRobloxWithRetry(
    MatchId: Uuid,
    Members: Vec<PartyMember>,
    State: Arc<AppState>,
) -> anyhow::Result<()> {
    let MaxRetries: u32 = 3;
    let mut CurrentAttempt: u32 = 0;
    let PlayerIds: Vec<u64> = Members.iter().map(|Member| Member.PlayerId).collect();

    let Config: &MatchmakingConfiguration = &State.MatchmakerInstance.Configuration;
    let Url: String = format!(
        "https://apis.roblox.com/messaging-service/v1/universes/{}/topics/{}",
        Config.RobloxUniverseId,
        Config.RobloxTopic
    );

    let MessageBody: String = json!({
        "MatchId": MatchId,
        "PlayerIds": PlayerIds
    }).to_string();

    let Payload: serde_json::Value = json!({ "message": MessageBody });

    loop {
        CurrentAttempt += 1;

        let Response = State.HttpClient
            .post(&Url)
            .header("x-api-key", &Config.RobloxApiKey)
            .header("Content-Type", "application/json")
            .json(&Payload)
            .send()
            .await;

        match Response {
            Ok(Res) if Res.status().is_success() => {
                info!(">>> Match {} notification sent to Roblox.", MatchId);
                return Ok(());
            }
            Ok(Res) => warn!("Attempt {} failed (Status {}): {}", CurrentAttempt, Res.status(), MatchId),
            Err(Error) => warn!("Attempt {} failed (Error): {}", CurrentAttempt, Error),
        }

        if CurrentAttempt >= MaxRetries {
            break;
        }

        sleep(Duration::from_secs(2u64.pow(CurrentAttempt))).await;
    }

    Err(anyhow::anyhow!("Failed to notify Roblox after {} attempts", MaxRetries))
}
