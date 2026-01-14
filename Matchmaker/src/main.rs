#![allow(dead_code)]
#![allow(non_snake_case)]

mod Types;
mod Configurations;
mod Matchmaker;

use axum::{
    extract::{State as AxumState, Request},
    routing::post,
    Json as AxumJson, Router,
    http::{StatusCode, HeaderMap},
    middleware::{self, Next},
    response::Response,
};
use chrono::Utc;
use std::sync::Arc;
use tokio::time::{interval, Duration, sleep};
use uuid::Uuid;
use serde_json::json;
use tracing::{info, warn, error};

use redis::Client;
use redis::aio::MultiplexedConnection;

use crate::Types::{MatchmakingTicket, PartyMember, MatchmakingConfiguration};
use crate::Configurations::GetStandardConfiguration;
use crate::Matchmaker::Matchmaker as MatchmakingLogic;

const REDIS_TICKET_KEY: &str = "MATCHMAKING_QUEUES";

struct AppState {
    pub RedisClient: Client,
    pub MatchmakerInstance: MatchmakingLogic,
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

    if AuthHeader != Data.MatchmakerInstance.Configuration.ServerAuthSecret {
        warn!("Unauthorized access attempt detected.");
        return Err(StatusCode::UNAUTHORIZED);
    }

    Ok(Next.run(Req).await)
}

async fn ShutdownSignal() {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to install CTRL+C signal handler");
    info!("Shutdown signal received. Closing gracefully...");
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    info!("Initializing Production Matchmaker Server...");

    let Config: MatchmakingConfiguration = GetStandardConfiguration();
    let RedisUrl: &str = "redis://127.0.0.1:6379";
    let RedisClient: Client = Client::open(RedisUrl).expect("Invalid Redis URL");

    match RedisClient.get_multiplexed_async_connection().await {
        Ok(_) => info!("Successfully connected to Redis at {}", RedisUrl),
        Err(Error) => {
            error!("CRITICAL: Could not connect to Redis. Ensure your Docker container is running.");
            error!("Error Details: {}", Error);
            return;
        }
    }

    let SharedState: Arc<AppState> = Arc::new(AppState {
        RedisClient,
        MatchmakerInstance: MatchmakingLogic::new(Config),
    });

    let LoopState: Arc<AppState> = SharedState.clone();
    tokio::spawn(async move {
        let TickRate: u64 = LoopState.MatchmakerInstance.Configuration.MatchmakerTickRateSeconds;
        let mut Interval: tokio::time::Interval = interval(Duration::from_secs(TickRate));

        loop {
            Interval.tick().await;

            let mut Connection: MultiplexedConnection = match LoopState.RedisClient.get_multiplexed_async_connection().await {
                Ok(Conn) => Conn,
                Err(Error) => {
                    error!("Redis Connection Error: {}", Error);
                    continue;
                }
            };

            let RawTickets: Vec<String> = redis::cmd("HVALS")
                .arg(REDIS_TICKET_KEY)
                .query_async(&mut Connection)
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
            let Matches: Vec<Vec<MatchmakingTicket>> = LoopState.MatchmakerInstance.FindMatches(Tickets.clone());

            for FoundMatch in Matches {
                let MatchId: Uuid = Uuid::new_v4();
                let Members: Vec<PartyMember> = FoundMatch.iter().flat_map(|Ticket| Ticket.Members.clone()).collect();
                let MatchedTicketIds: Vec<String> = FoundMatch.iter().map(|Ticket| Ticket.TicketId.to_string()).collect();

                let InternalState: Arc<AppState> = LoopState.clone();
                tokio::spawn(async move {
                    if let Err(Error) = SendMatchToRobloxWithRetry(MatchId, Members, InternalState.clone()).await {
                        error!("FATAL: Match notification failed: {}", Error);
                    } else {
                        let mut Con: MultiplexedConnection = InternalState.RedisClient.get_multiplexed_async_connection().await.unwrap();
                        let _: () = redis::cmd("HDEL")
                            .arg(REDIS_TICKET_KEY)
                            .arg(MatchedTicketIds)
                            .query_async(&mut Con)
                            .await
                            .unwrap_or_default();
                    }
                });
            }

            for Ticket in Tickets {
                let SerializedTicket: String = serde_json::to_string(&Ticket).unwrap();
                let _: () = redis::cmd("HSET")
                    .arg(REDIS_TICKET_KEY)
                    .arg(Ticket.TicketId.to_string())
                    .arg(SerializedTicket)
                    .query_async(&mut Connection)
                    .await
                    .unwrap_or_default();
            }
        }
    });

    let App: Router = Router::new()
        .route("/submit", post(SubmitTicket))
        .layer(middleware::from_fn_with_state(SharedState.clone(), AuthMiddleware))
        .with_state(SharedState);

    let Addr: &str = "0.0.0.0:3000";
    let Listener: tokio::net::TcpListener = tokio::net::TcpListener::bind(Addr).await.unwrap();
    info!("Server Online. Listening on http://{}", Addr);

    axum::serve(Listener, App)
        .with_graceful_shutdown(ShutdownSignal())
        .await
        .unwrap();
}

async fn SubmitTicket(
    AxumState(Data): AxumState<Arc<AppState>>,
    AxumJson(Members): AxumJson<Vec<PartyMember>>,
) -> Result<AxumJson<Uuid>, StatusCode> {
    if Members.len() > Data.MatchmakerInstance.Configuration.MaximumPartySize as usize {
        warn!("Rejected ticket: Party size {} exceeds maximum", Members.len());
        return Err(StatusCode::BAD_REQUEST);
    }

    let TicketId: Uuid = Uuid::new_v4();
    let AverageRating: f64 = Members.iter().map(|Member| Member.MatchmakingRating).sum::<f64>() / Members.len() as f64;
    let InitialRange: f64 = Data.MatchmakerInstance.Configuration.InitialMatchmakingRatingRange;

    let NewTicket: MatchmakingTicket = MatchmakingTicket {
        TicketId,
        Members,
        AverageMatchmakingRating: AverageRating,
        PreferredRegion: "NorthAmerica".to_string(),
        AllowedRegions: vec!["NorthAmerica".to_string()],
        SubmittedTimestamp: Utc::now().timestamp(),
        SearchExpansionLevel: 0,
        MinimumMatchmakingRating: AverageRating - InitialRange,
        MaximumMatchmakingRating: AverageRating + InitialRange,
    };

    let mut Connection: MultiplexedConnection = Data
        .RedisClient
        .get_multiplexed_async_connection()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let SerializedTicket: String = serde_json::to_string(&NewTicket).unwrap();
    let _: () = redis::cmd("HSET")
        .arg(REDIS_TICKET_KEY)
        .arg(TicketId.to_string())
        .arg(SerializedTicket)
        .query_async(&mut Connection)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    info!("+ Party Ticket Queued: {} (Size: {})", TicketId, NewTicket.Members.len());
    Ok(AxumJson(TicketId))
}

async fn SendMatchToRobloxWithRetry(
    MatchId: Uuid,
    Members: Vec<PartyMember>,
    InternalAppState: Arc<AppState>,
) -> anyhow::Result<()> {
    let MaxRetries: u32 = 3;
    let mut CurrentAttempt: u32 = 0;
    let HttpClient: reqwest::Client = reqwest::Client::new();
    let PlayerIds: Vec<u64> = Members.iter().map(|Member| Member.PlayerId).collect();

    let Config: &MatchmakingConfiguration = &InternalAppState.MatchmakerInstance.Configuration;
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

        let Response = HttpClient
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
