#![allow( dead_code )]
#![allow( non_snake_case )]

mod Types;
mod Configurations;
mod Matchmaker;

use axum :: {
    extract :: State,
    routing :: post,
    Json , Router,
};
use chrono :: Utc;
use std::sync :: {Arc , Mutex};
use tokio::time :: {interval , Duration};
use uuid :: Uuid;
use serde_json :: json;
use tracing :: {info , warn , error}; 

use crate::Types :: {MatchmakingTicket , PartyMember};
use crate::Configurations :: GetStandardConfiguration;

struct AppState {
    pub Tickets : Mutex <Vec <MatchmakingTicket >>,
    pub MatchmakerInstance : Matchmaker::Matchmaker,
}

async fn ShutdownSignal() {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to install CTRL+C signal handler");
    info!("Shutdown signal received. Closing Matchmaker server...");
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    info!("Initializing Matchmaker Server...");

    let SharedState : Arc <AppState > = Arc::new(AppState {
        Tickets : Mutex::new(Vec::new()),
        MatchmakerInstance : Matchmaker::Matchmaker::new(GetStandardConfiguration()),
    });

    let LoopState : Arc <AppState > = SharedState.clone();
    
    tokio::spawn(async move {
        let TickRate : u64 = LoopState.MatchmakerInstance.Configuration.MatchmakerTickRateSeconds;
        let mut Interval : tokio::time::Interval = interval(Duration::from_secs(TickRate));
        
        info!("Matchmaker Loop Started. Tick Rate: {}s", TickRate);

        loop {
            Interval.tick().await;
            
            let mut Tickets : std::sync::MutexGuard <Vec <MatchmakingTicket >> = LoopState.Tickets.lock().unwrap();
            
            if !Tickets.is_empty() {
                info!("--- Tick: Checking {} tickets in queue ---", Tickets.len());
            }

            LoopState.MatchmakerInstance.ExpandTickets(&mut Tickets);
            let Matches : Vec <Vec <MatchmakingTicket >> = LoopState.MatchmakerInstance.FindMatches(Tickets.clone());

            if !Matches.is_empty() {
                info!("!!! FOUND {} MATCHES !!!", Matches.len());
            }

            for Match in Matches {
                let MatchId : Uuid = Uuid::new_v4();
                let PlayerCount : usize = Match.iter().map(|T| T.Members.len()).sum::<usize >();
                
                info!(">>> Creating Match {} | Players: {}", MatchId , PlayerCount);

                let AllMembers : Vec <PartyMember > = Match.iter()
                    .flat_map(|T| T.Members.clone())
                    .collect();

                tokio::spawn(async move {
                    if let Err(E) = SendMatchToRoblox(MatchId , AllMembers).await {
                        error!("Failed to notify Roblox: {}", E);
                    }
                });

                let MatchedIds : Vec <Uuid > = Match.iter().map(|T| T.TicketId).collect();
                Tickets.retain(|T| !MatchedIds.contains(&T.TicketId));
            }
        }
    });

    let App : Router = Router::new()
        .route("/submit" , post(SubmitTicket))
        .with_state(SharedState);

    let Addr : &str = "0.0.0.0:3000";
    let Listener : tokio::net::TcpListener = tokio::net::TcpListener::bind(Addr).await.unwrap();
    
    info!("Server Online. Listening on {}", Addr);
    
    // Graceful shutdown implementation
    axum::serve(Listener , App)
        .with_graceful_shutdown(ShutdownSignal())
        .await
        .unwrap();

    info!("Server has shut down gracefully.");
}

async fn SubmitTicket(
    State(Data) : State <Arc <AppState >>,
    Json(Members) : Json <Vec <PartyMember >>
) -> Json <Uuid > {
    let TicketId : Uuid = Uuid::new_v4();
    
    let PlayerNames : Vec <String > = Members.iter().map(|M| M.PlayerName.clone()).collect();
    info!("+ New Request: {} (Members: {:?})", TicketId , PlayerNames);

    let AvgRating : f64 = Members.iter().map(|M| M.MatchmakingRating).sum::<f64 >() / Members.len() as f64;
    
    let NewTicket : MatchmakingTicket = MatchmakingTicket {
        TicketId,
        Members,
        AverageMatchmakingRating : AvgRating,
        PreferredRegion : "NorthAmerica".to_string(),
        AllowedRegions : vec!["NorthAmerica".to_string()],
        SubmittedTimestamp : Utc::now().timestamp(),
        SearchExpansionLevel : 0,
        MinimumMatchmakingRating : AvgRating - 100.0,
        MaximumMatchmakingRating : AvgRating + 100.0,
    };

    let mut Lock : std::sync::MutexGuard <Vec <MatchmakingTicket >> = Data.Tickets.lock().unwrap();
    Lock.push(NewTicket);
    info!("  -> Added to Queue. Current Queue Size: {}", Lock.len());

    Json(TicketId)
}

async fn SendMatchToRoblox(MatchId : Uuid , Members : Vec <PartyMember >) -> anyhow::Result <()> {
    let UniverseId : &str = "9504219975"; 
    let ApiKey : &str = "YOUR_API_KEY"; 
    let Topic : &str = "GlobalMatchmaking"; 

    info!("    -> Sending Match {} to Roblox Cloud...", MatchId);

    let Client : reqwest::Client = reqwest::Client::new();
    let PlayerIds : Vec <u64 > = Members.iter().map(|M| M.PlayerId).collect();

    let Payload : serde_json::Value = json!({
        "message": json!({
            "MatchId": MatchId,
            "PlayerIds": PlayerIds
        }).to_string()
    });

    let Url : String = format!("https://apis.roblox.com/messaging-service/v1/universes/{}/topics/{}" , UniverseId , Topic);

    let Response : reqwest::Response = Client.post(&Url)
        .header("x-api-key" , ApiKey)
        .header("Content-Type" , "application/json")
        .json(&Payload)
        .send()
        .await?; 

    if !Response.status().is_success() {
        let Status : reqwest::StatusCode = Response.status();
        let Body : String = Response.text().await.unwrap_or_default();
        warn!("    !! ERROR from Roblox: Status {}" , Status);
        warn!("    !! Body: {}" , Body);
        return Err(anyhow::anyhow!("Roblox API returned error {}" , Status));
    }

    info!("    -> SUCCESS: Roblox received Match {}" , MatchId);
    Ok(())
}