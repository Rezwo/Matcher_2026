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
use dashmap::DashMap;
use governor::{Quota, RateLimiter};
use governor::clock::DefaultClock;
use governor::state::{InMemoryState, NotKeyed};
use rayon::prelude::*;
use std::collections::HashSet;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::time::{interval, Duration, sleep};
use tokio::sync::Notify;
use uuid::Uuid;
use serde_json::json;
use tracing::{info, warn, error};
use subtle::ConstantTimeEq;

use crate::Types::{
    MatchmakingTicket, PartyMember, MatchmakingConfiguration, PartyMembers,
    SubmitTicketRequest, HealthResponse, ErrorResponse,
    TicketStatusResponse, MetricsSnapshot, MatchmakerMetrics, TicketStatus
};
use crate::Configurations::GetStandardConfiguration;
use crate::Matchmaker::Matchmaker as MatchmakingLogic;

const REDIS_TICKET_KEY: &str = "MATCHMAKING_QUEUES";
const REDIS_QUEUE_ZSET: &str = "MATCHMAKING_QUEUE_ORDER";

type SubmitRateLimiter = RateLimiter<NotKeyed, InMemoryState, DefaultClock>;

struct AppState {
    pub RedisPool: Pool<RedisConnectionManager>,
    pub HttpClient: reqwest::Client,
    pub MatchmakerInstance: MatchmakingLogic,
    pub ShuttingDown: AtomicBool,
    pub ShutdownNotify: Notify,
    pub Metrics: MatchmakerMetrics,
    pub CurrentQueueSize: AtomicU64,
    pub QueuePositions: DashMap<Uuid, usize>,
    pub SubmitLimiter: SubmitRateLimiter,
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
    info!("Metrics available at /metrics/snapshot");

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

    let SubmitLimiter = RateLimiter::direct(Quota::per_second(NonZeroU32::new(100).unwrap()));

    let SharedState: Arc<AppState> = Arc::new(AppState {
        RedisPool,
        HttpClient,
        MatchmakerInstance: MatchmakingLogic::new(Config),
        ShuttingDown: AtomicBool::new(false),
        ShutdownNotify: Notify::new(),
        Metrics: MatchmakerMetrics::new(),
        CurrentQueueSize: AtomicU64::new(0),
        QueuePositions: DashMap::new(),
        SubmitLimiter,
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

            let mut RawTickets: Vec<String> = Vec::with_capacity(256);
            let mut Cursor: u64 = 0;
            loop {
                let ScanResult: (u64, Vec<(String, String)>) = redis::cmd("HSCAN")
                    .arg(REDIS_TICKET_KEY)
                    .arg(Cursor)
                    .arg("COUNT")
                    .arg(100)
                    .query_async(&mut *Connection)
                    .await
                    .unwrap_or((0, Vec::new()));

                Cursor = ScanResult.0;
                for (_, Value) in ScanResult.1 {
                    RawTickets.push(Value);
                }

                if Cursor == 0 {
                    break;
                }
            }

            let AllTickets: Vec<MatchmakingTicket> = RawTickets
                .par_iter()
                .filter_map(|Serialized| serde_json::from_str::<MatchmakingTicket>(Serialized).ok())
                .collect();

            let mut Tickets: Vec<MatchmakingTicket> = AllTickets
                .into_iter()
                .filter(|T| T.Status == TicketStatus::Queued)
                .collect();

            LoopState.CurrentQueueSize.store(Tickets.len() as u64, Ordering::Relaxed);

            if Tickets.is_empty() {
                LoopState.QueuePositions.clear();
                continue;
            }

            let ExpiredTickets = LoopState.MatchmakerInstance.RemoveExpiredTickets(&mut Tickets);
            if !ExpiredTickets.is_empty() {
                let ExpiredCount = ExpiredTickets.len();
                LoopState.Metrics.TotalTicketsExpired.fetch_add(ExpiredCount as u64, Ordering::Relaxed);

                let ExpiredIds: Vec<String> = ExpiredTickets.iter().map(|T| T.TicketId.to_string()).collect();
                let _: Result<(), _> = redis::cmd("HDEL")
                    .arg(REDIS_TICKET_KEY)
                    .arg(ExpiredIds)
                    .query_async(&mut *Connection)
                    .await;

                info!("Expired {} stale tickets", ExpiredCount);
            }

            if Tickets.is_empty() {
                LoopState.QueuePositions.clear();
                continue;
            }

            LoopState.MatchmakerInstance.ExpandTickets(&mut Tickets);

            Tickets.sort_by(|A, B| A.SubmittedTimestamp.cmp(&B.SubmittedTimestamp));

            LoopState.QueuePositions.clear();
            for (Index, Ticket) in Tickets.iter().enumerate() {
                LoopState.QueuePositions.insert(Ticket.TicketId, Index + 1);
            }

            let Matches: Vec<Vec<MatchmakingTicket>> = LoopState.MatchmakerInstance.FindMatches(&Tickets);
            let MatchedIds: HashSet<Uuid> = LoopState.MatchmakerInstance.GetMatchedTicketIds(&Matches);

            let CurrentTimestamp = Utc::now().timestamp();

            for FoundMatch in &Matches {
                let MatchId: Uuid = Uuid::new_v4();
                let MatchMembers: Vec<PartyMember> = FoundMatch.iter().flat_map(|Ticket| Ticket.Members.iter().cloned()).collect();
                let MatchedTicketIds: Vec<String> = FoundMatch.iter().map(|Ticket| Ticket.TicketId.to_string()).collect();

                for Ticket in FoundMatch {
                    let WaitTime = CurrentTimestamp.saturating_sub(Ticket.SubmittedTimestamp) as u64;
                    LoopState.Metrics.TotalWaitTimeSeconds.fetch_add(WaitTime, Ordering::Relaxed);
                    LoopState.Metrics.MatchedTicketCount.fetch_add(1, Ordering::Relaxed);
                }

                {
                    let mut Pipeline = redis::pipe();
                    for Ticket in FoundMatch {
                        let mut MarkedTicket = Ticket.clone();
                        MarkedTicket.Status = TicketStatus::Matched;
                        if let Ok(SerializedTicket) = serde_json::to_string(&MarkedTicket) {
                            Pipeline.cmd("HSET")
                                .arg(REDIS_TICKET_KEY)
                                .arg(Ticket.TicketId.to_string())
                                .arg(SerializedTicket)
                                .ignore();
                        }
                    }
                    let _: Result<(), _> = Pipeline.query_async(&mut *Connection).await;
                }

                for Ticket in FoundMatch {
                    LoopState.QueuePositions.remove(&Ticket.TicketId);
                }

                let InternalState: Arc<AppState> = LoopState.clone();
                tokio::spawn(async move {
                    match SendMatchToRobloxWithRetry(MatchId, MatchMembers, InternalState.clone()).await {
                        Ok(_) => {
                            InternalState.Metrics.TotalMatchesCreated.fetch_add(1, Ordering::Relaxed);

                            if let Ok(mut Con) = InternalState.RedisPool.get().await {
                                let _: Result<(), _> = redis::cmd("HDEL")
                                    .arg(REDIS_TICKET_KEY)
                                    .arg(MatchedTicketIds.clone())
                                    .query_async(&mut *Con)
                                    .await;
                            }
                        }
                        Err(Error) => {
                            error!("FATAL: Match notification failed for {}: {}", MatchId, Error);
                            if let Ok(mut Con) = InternalState.RedisPool.get().await {
                                let _: Result<(), _> = redis::cmd("HDEL")
                                    .arg(REDIS_TICKET_KEY)
                                    .arg(MatchedTicketIds)
                                    .query_async(&mut *Con)
                                    .await;
                            }
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
        .route("/ticket/{TicketId}", get(GetTicketStatus))
        .route("/ticket/{TicketId}", delete(CancelTicket))
        .layer(middleware::from_fn_with_state(SharedState.clone(), AuthMiddleware));

    let App: Router = Router::new()
        .route("/health", get(HealthCheck))
        .route("/metrics/snapshot", get(GetMetricsSnapshot))
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

async fn GetMetricsSnapshot(
    AxumState(Data): AxumState<Arc<AppState>>,
) -> impl IntoResponse {
    let Snapshot = MetricsSnapshot {
        QueueSize: Data.CurrentQueueSize.load(Ordering::Relaxed),
        TotalTicketsProcessed: Data.Metrics.TotalTicketsProcessed.load(Ordering::Relaxed),
        TotalMatchesCreated: Data.Metrics.TotalMatchesCreated.load(Ordering::Relaxed),
        TotalTicketsExpired: Data.Metrics.TotalTicketsExpired.load(Ordering::Relaxed),
        AverageWaitTimeSeconds: Data.Metrics.GetAverageWaitTime(),
        MatchesLastMinute: 0,
    };

    (StatusCode::OK, AxumJson(Snapshot))
}

async fn GetTicketStatus(
    AxumState(Data): AxumState<Arc<AppState>>,
    Path(TicketId): Path<Uuid>,
) -> Result<AxumJson<TicketStatusResponse>, (StatusCode, AxumJson<ErrorResponse>)> {
    let mut Connection = Data.RedisPool.get().await.map_err(|_| (
        StatusCode::INTERNAL_SERVER_ERROR,
        AxumJson(ErrorResponse { Error: "Failed to connect to Redis".to_string() }),
    ))?;

    let TicketJson: Option<String> = redis::cmd("HGET")
        .arg(REDIS_TICKET_KEY)
        .arg(TicketId.to_string())
        .query_async(&mut *Connection)
        .await
        .map_err(|_| (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(ErrorResponse { Error: "Failed to query Redis".to_string() }),
        ))?;

    let TicketJson = TicketJson.ok_or((
        StatusCode::NOT_FOUND,
        AxumJson(ErrorResponse { Error: "Ticket not found".to_string() }),
    ))?;

    let Ticket: MatchmakingTicket = serde_json::from_str(&TicketJson).map_err(|_| (
        StatusCode::INTERNAL_SERVER_ERROR,
        AxumJson(ErrorResponse { Error: "Failed to parse ticket data".to_string() }),
    ))?;

    let TicketStatusStr = match Ticket.Status {
        TicketStatus::Queued => "queued",
        TicketStatus::Matched => "matched",
    };

    let CurrentTimestamp = Utc::now().timestamp();
    let WaitTimeSeconds = CurrentTimestamp.saturating_sub(Ticket.SubmittedTimestamp);

    let QueuePosition = Data.QueuePositions.get(&TicketId).map(|V| *V);

    let EstimatedWaitSeconds = if Data.Metrics.GetAverageWaitTime() > 0.0 {
        Some((Data.Metrics.GetAverageWaitTime() * QueuePosition.unwrap_or(1) as f64) as i64)
    } else {
        None
    };

    let Response = TicketStatusResponse {
        TicketId,
        Status: TicketStatusStr.to_string(),
        QueuePosition,
        WaitTimeSeconds,
        ExpansionLevel: Ticket.SearchExpansionLevel,
        EstimatedWaitSeconds,
    };

    Ok(AxumJson(Response))
}

async fn SubmitTicket(
    AxumState(Data): AxumState<Arc<AppState>>,
    AxumJson(Request): AxumJson<SubmitTicketRequest>,
) -> Result<AxumJson<Uuid>, (StatusCode, AxumJson<ErrorResponse>)> {
    if Data.SubmitLimiter.check().is_err() {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            AxumJson(ErrorResponse { Error: "Rate limit exceeded. Try again later.".to_string() }),
        ));
    }

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

    let Members: PartyMembers = Request.Members.into_iter().collect();

    let NewTicket: MatchmakingTicket = MatchmakingTicket {
        TicketId,
        Members,
        AverageMatchmakingRating: AverageRating,
        PreferredRegion: PreferredRegion.clone(),
        AllowedRegions,
        SubmittedTimestamp: Utc::now().timestamp(),
        SearchExpansionLevel: 0,
        MinimumMatchmakingRating: AverageRating - InitialRange,
        MaximumMatchmakingRating: AverageRating + InitialRange,
        Status: TicketStatus::Queued,
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

    Data.Metrics.TotalTicketsProcessed.fetch_add(1, Ordering::Relaxed);

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

 // >>>>>>>>> PASTE HERE <<<<<<<<<
    info!("DEBUG: Sending to URL: {}", Url);
    info!("DEBUG: Payload: {}", Payload.to_string());
    // Check if the key exists and print its length (don't print the actual key for security)
    info!("DEBUG: API Key Length: {}", Config.RobloxApiKey.len()); 

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
