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
use governor::state::{InMemoryState, NotKeyed, keyed::DashMapStateStore};
use std::collections::{HashMap, HashSet};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::time::{interval, Duration, sleep};
use tokio::sync::Notify;
use uuid::Uuid;
use serde_json::json;
use tracing::{info, warn, error, debug};
use subtle::ConstantTimeEq;

use crate::Types::{
    MatchmakingTicket, PartyMember, MatchmakingConfiguration, PartyMembers,
    SubmitTicketRequest, HealthResponse, ErrorResponse,
    TicketStatusResponse, MetricsSnapshot, MatchmakerMetrics, TicketStatus, GameMode,
};
use crate::Configurations::GetStandardConfiguration;
use crate::Matchmaker::Matchmaker as MatchmakingLogic;

const REDIS_TICKET_KEY: &str = "MATCHMAKING_QUEUES";

type SubmitRateLimiter = RateLimiter<NotKeyed, InMemoryState, DefaultClock>;
type RegionalRateLimiter = RateLimiter<String, DashMapStateStore<String>, DefaultClock>;

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
    pub RegionalLimiter: RegionalRateLimiter,
    pub RegionalQueueSizes: DashMap<String, u64>,
    pub DebugMode: bool,
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
) -> Result<Response, (StatusCode, AxumJson<ErrorResponse>)> {
    let AuthHeader = Headers.get("X-Api-Key");

    // Check if header exists
    if AuthHeader.is_none() {
        warn!("Unauthorized: Missing X-Api-Key header");
        return Err((
            StatusCode::UNAUTHORIZED,
            AxumJson(ErrorResponse { Error: "Missing X-Api-Key header".to_string() }),
        ));
    }

    // Check if header is valid UTF-8
    let AuthValue = AuthHeader
        .unwrap()
        .to_str()
        .map_err(|_| {
            warn!("Unauthorized: Invalid X-Api-Key header encoding");
            (
                StatusCode::UNAUTHORIZED,
                AxumJson(ErrorResponse { Error: "Invalid X-Api-Key header encoding".to_string() }),
            )
        })?;

    // Constant-time comparison
    if !ConstantTimeCompare(AuthValue, &Data.MatchmakerInstance.Configuration.ServerAuthSecret) {
        warn!("Unauthorized: Invalid API key");
        return Err((
            StatusCode::UNAUTHORIZED,
            AxumJson(ErrorResponse { Error: "Invalid API key".to_string() }),
        ));
    }

    Ok(Next.run(Req).await)
}

async fn ShutdownSignal(State: Arc<AppState>) {
    match tokio::signal::ctrl_c().await {
        Ok(_) => {
            info!("Shutdown signal received. Draining queue...");
            State.ShuttingDown.store(true, Ordering::SeqCst);
            State.ShutdownNotify.notified().await;
            info!("Queue drained. Closing gracefully...");
        }
        Err(Error) => {
            error!("Failed to listen for shutdown signal: {}", Error);
        }
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    info!("Initializing Production Matchmaker Server...");
    info!("Metrics available at /metrics/snapshot");

    let Config: MatchmakingConfiguration = GetStandardConfiguration();
    let DebugMode = Config.DebugMode;
    let RedisUrl: String = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

    let Manager: RedisConnectionManager = match RedisConnectionManager::new(RedisUrl.clone()) {
        Ok(M) => M,
        Err(Error) => {
            error!("Invalid Redis URL '{}': {}", RedisUrl, Error);
            return;
        }
    };

    let RedisPool: Pool<RedisConnectionManager> = match Pool::builder()
        .max_size(16)
        .min_idle(Some(4))
        .build(Manager)
        .await
    {
        Ok(Pool) => Pool,
        Err(Error) => {
            error!("Failed to create Redis connection pool: {}", Error);
            return;
        }
    };

    match RedisPool.get().await {
        Ok(_) => info!("Successfully connected to Redis at {}", RedisUrl),
        Err(Error) => {
            error!("CRITICAL: Could not connect to Redis. Ensure your Docker container is running.");
            error!("Error Details: {}", Error);
            return;
        }
    }

    let HttpClient: reqwest::Client = match reqwest::Client::builder()
        .pool_max_idle_per_host(10)
        .timeout(Duration::from_secs(30))
        .build()
    {
        Ok(Client) => Client,
        Err(Error) => {
            error!("Failed to create HTTP client: {}", Error);
            return;
        }
    };

    let SubmitLimiter = RateLimiter::direct(Quota::per_second(NonZeroU32::new(1000).unwrap()));
    let RegionalLimiter = RateLimiter::dashmap(Quota::per_second(NonZeroU32::new(500).unwrap()));

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
        RegionalLimiter,
        RegionalQueueSizes: DashMap::new(),
        DebugMode,
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

            // Scan tickets from Redis with proper error handling
            let mut RawTickets: Vec<String> = Vec::with_capacity(256);
            let mut Cursor: u64 = 0;
            let mut ScanFailed = false;

            loop {
                let ScanResult: Result<(u64, Vec<(String, String)>), redis::RedisError> = redis::cmd("HSCAN")
                    .arg(REDIS_TICKET_KEY)
                    .arg(Cursor)
                    .arg("COUNT")
                    .arg(100)
                    .query_async(&mut *Connection)
                    .await;

                match ScanResult {
                    Ok((NewCursor, Entries)) => {
                        Cursor = NewCursor;
                        for (_, Value) in Entries {
                            RawTickets.push(Value);
                        }
                        if Cursor == 0 {
                            break;
                        }
                    }
                    Err(Error) => {
                        error!("Redis HSCAN failed: {}. Skipping this tick.", Error);
                        ScanFailed = true;
                        break;
                    }
                }
            }

            if ScanFailed {
                continue;
            }

            // Parse tickets with logging for failures
            let mut ParseFailures = 0u32;
            let AllTickets: Vec<MatchmakingTicket> = RawTickets
                .iter()
                .filter_map(|Serialized| {
                    match serde_json::from_str::<MatchmakingTicket>(Serialized) {
                        Ok(Ticket) => Some(Ticket),
                        Err(Error) => {
                            ParseFailures += 1;
                            if LoopState.DebugMode {
                                debug!("Failed to parse ticket: {}", Error);
                            }
                            None
                        }
                    }
                })
                .collect();

            if ParseFailures > 0 {
                warn!("Failed to parse {} tickets from Redis", ParseFailures);
            }

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
                match redis::cmd("HDEL")
                    .arg(REDIS_TICKET_KEY)
                    .arg(&ExpiredIds)
                    .query_async::<()>(&mut *Connection)
                    .await
                {
                    Ok(_) => info!("Expired {} stale tickets", ExpiredCount),
                    Err(Error) => warn!("Failed to delete expired tickets: {}", Error),
                }
            }

            if Tickets.is_empty() {
                LoopState.QueuePositions.clear();
                continue;
            }

            LoopState.MatchmakerInstance.ExpandTickets(&mut Tickets);

            // Sort by priority (higher first), then by timestamp (earlier first)
            Tickets.sort_by(|A, B| {
                B.Priority.cmp(&A.Priority)
                    .then_with(|| A.SubmittedTimestamp.cmp(&B.SubmittedTimestamp))
            });

            // Update queue positions atomically
            {
                LoopState.QueuePositions.clear();
                for (Index, Ticket) in Tickets.iter().enumerate() {
                    LoopState.QueuePositions.insert(Ticket.TicketId, Index + 1);
                }
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

                // Mark tickets as matched with error handling
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
                    if let Err(Error) = Pipeline.query_async::<()>(&mut *Connection).await {
                        error!("Failed to mark tickets as matched: {}", Error);
                    }
                }

                for Ticket in FoundMatch {
                    LoopState.QueuePositions.remove(&Ticket.TicketId);
                }

                let InternalState: Arc<AppState> = LoopState.clone();
                let MatchIdClone = MatchId;
                let TicketIdsClone = MatchedTicketIds.clone();

                tokio::spawn(async move {
                    match SendMatchToRobloxWithRetry(MatchIdClone, MatchMembers, InternalState.clone()).await {
                        Ok(_) => {
                            InternalState.Metrics.TotalMatchesCreated.fetch_add(1, Ordering::Relaxed);

                            match InternalState.RedisPool.get().await {
                                Ok(mut Con) => {
                                    if let Err(Error) = redis::cmd("HDEL")
                                        .arg(REDIS_TICKET_KEY)
                                        .arg(&TicketIdsClone)
                                        .query_async::<()>(&mut *Con)
                                        .await
                                    {
                                        error!("Failed to delete matched tickets {}: {}", MatchIdClone, Error);
                                    }
                                }
                                Err(Error) => {
                                    error!("Failed to get Redis connection for cleanup: {}", Error);
                                }
                            }
                        }
                        Err(Error) => {
                            error!("FATAL: Match notification failed for {}: {}", MatchIdClone, Error);
                            // Still try to clean up the tickets to prevent them from being stuck
                            match InternalState.RedisPool.get().await {
                                Ok(mut Con) => {
                                    if let Err(DelError) = redis::cmd("HDEL")
                                        .arg(REDIS_TICKET_KEY)
                                        .arg(&TicketIdsClone)
                                        .query_async::<()>(&mut *Con)
                                        .await
                                    {
                                        error!("Failed to cleanup failed match tickets: {}", DelError);
                                    }
                                }
                                Err(ConnError) => {
                                    error!("Failed to get Redis connection for failed match cleanup: {}", ConnError);
                                }
                            }
                        }
                    }
                });
            }

            // Update unmatched tickets with expanded search ranges
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
                if let Err(Error) = Pipeline.query_async::<()>(&mut *Connection).await {
                    warn!("Failed to update unmatched tickets: {}", Error);
                }
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
    let Listener = match tokio::net::TcpListener::bind(Addr).await {
        Ok(L) => L,
        Err(Error) => {
            error!("Failed to bind to {}: {}", Addr, Error);
            return;
        }
    };
    info!("Server Online. Listening on http://{}", Addr);

    let ShutdownState = SharedState.clone();
    if let Err(Error) = axum::serve(Listener, App)
        .with_graceful_shutdown(ShutdownSignal(ShutdownState))
        .await
    {
        error!("Server error: {}", Error);
    }
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
        ModeMetrics: HashMap::new(),
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

    // Only estimate wait time if we have a valid queue position
    let EstimatedWaitSeconds = match QueuePosition {
        Some(Position) if Data.Metrics.GetAverageWaitTime() > 0.0 => {
            Some((Data.Metrics.GetAverageWaitTime() * Position as f64) as i64)
        }
        _ => None,
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
            AxumJson(ErrorResponse { Error: "Global rate limit exceeded. Try again later.".to_string() }),
        ));
    }

    let Config = &Data.MatchmakerInstance.Configuration;

    // Validate region for rate limiting - use provided region or default
    let PreferredRegionForLimit: String = Request.PreferredRegion
        .as_ref()
        .filter(|R| Config.ValidRegions.contains(*R))
        .cloned()
        .unwrap_or_else(|| Config.DefaultRegion.clone());

    if Data.RegionalLimiter.check_key(&PreferredRegionForLimit).is_err() {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            AxumJson(ErrorResponse {
                Error: format!("Regional rate limit exceeded for {}. Try again later.", PreferredRegionForLimit)
            }),
        ));
    }

    // Validate members array
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

    // Validate each member
    for (Index, Member) in Request.Members.iter().enumerate() {
        if Member.PlayerId == 0 {
            return Err((
                StatusCode::BAD_REQUEST,
                AxumJson(ErrorResponse { Error: format!("Member {} has invalid PlayerId (0)", Index) }),
            ));
        }
        if Member.PlayerName.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                AxumJson(ErrorResponse { Error: format!("Member {} has empty PlayerName", Index) }),
            ));
        }
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

    // Validate GameMode if provided
    let RequestedGameMode: GameMode = Request.GameMode.unwrap_or_default();
    if !MatchmakingLogic::ValidateGameMode(&RequestedGameMode) {
        return Err((
            StatusCode::BAD_REQUEST,
            AxumJson(ErrorResponse {
                Error: "Invalid GameMode: Name cannot be empty and MinPlayers must be > 0 and <= MaxPlayers".to_string()
            }),
        ));
    }

    // Validate Priority is reasonable
    if Request.Priority > 10000 {
        return Err((
            StatusCode::BAD_REQUEST,
            AxumJson(ErrorResponse { Error: "Priority cannot exceed 10000".to_string() }),
        ));
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
        GameMode: RequestedGameMode,
        Priority: Request.Priority,
        CustomData: Request.CustomData,
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

    info!("+ Party Ticket Queued: {} (Size: {}, Region: {}, Mode: {}, Priority: {})",
        TicketId, NewTicket.Members.len(), PreferredRegion, NewTicket.GameMode.Name, NewTicket.Priority);
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
        Data.QueuePositions.remove(&TicketId);
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
                info!(">>> Match {} notification sent to Roblox ({} players).", MatchId, PlayerIds.len());
                return Ok(());
            }
            Ok(Res) => {
                let Status = Res.status();
                let Body = Res.text().await.unwrap_or_default();
                warn!("Attempt {}/{} failed for match {} (Status {}): {}",
                    CurrentAttempt, MaxRetries, MatchId, Status, Body);
            }
            Err(Error) => {
                warn!("Attempt {}/{} failed for match {} (Error): {}",
                    CurrentAttempt, MaxRetries, MatchId, Error);
            }
        }

        if CurrentAttempt >= MaxRetries {
            break;
        }

        sleep(Duration::from_secs(2u64.pow(CurrentAttempt))).await;
    }

    Err(anyhow::anyhow!("Failed to notify Roblox after {} attempts for match {}", MaxRetries, MatchId))
}
