#![allow( dead_code )]
#![allow(non_snake_case)]

mod Types;
mod Configurations;
mod Matchmaker;

use axum::{
    extract::State,
    routing::post,
    Json, Router,
};
use chrono::Utc;
use std::sync::{Arc, Mutex};
use tokio::time::{interval, Duration};
use uuid::Uuid;
use serde_json::json; 

// --- FIX: DO NOT USE Matchmaker::Matchmaker HERE ---
use crate::Types::{MatchmakingTicket, PartyMember};
use crate::Configurations::GetStandardConfiguration;

struct AppState {
    pub Tickets : Mutex<Vec<MatchmakingTicket>>,
    // FIX: Refer to it as Module::Struct
    pub MatchmakerInstance : Matchmaker::Matchmaker,
}

#[tokio::main]
async fn main() {
    let SharedState = Arc::new(AppState {
        Tickets : Mutex::new(Vec::new()),
        // FIX: Call new() on the fully qualified path
        MatchmakerInstance : Matchmaker::Matchmaker::new(GetStandardConfiguration()),
    });

    // Background Matchmaking Loop
    let LoopState = SharedState.clone();
    
    tokio::spawn(async move {
        let TickRate = LoopState.MatchmakerInstance.Configuration.MatchmakerTickRateSeconds;
        let mut Interval = interval(Duration::from_secs(TickRate));
        
        loop {
            Interval.tick().await;
            
            let mut Tickets = LoopState.Tickets.lock().unwrap();
            
            // 1. Expand Searches
            LoopState.MatchmakerInstance.ExpandTickets(&mut Tickets);

            // 2. Attempt Matching
            let Matches = LoopState.MatchmakerInstance.FindMatches(Tickets.clone());

            for Match in Matches {
                let MatchId : Uuid = Uuid::new_v4();
                let PlayerCount : usize = Match.iter().map(|T| T.Members.len()).sum();
                
                println!("Match Created: {} with {} players", MatchId, PlayerCount);

                // --- ROBLOX COMMUNICATION ---
                let AllMembers : Vec<PartyMember> = Match.iter()
                    .flat_map(|T| T.Members.clone())
                    .collect();

                tokio::spawn(async move {
                    SendMatchToRoblox(MatchId, AllMembers).await;
                });
                // ---------------------------

                let MatchedIds : Vec<Uuid> = Match.iter().map(|T| T.TicketId).collect();
                Tickets.retain(|T| !MatchedIds.contains(&T.TicketId));
            }
        }
    });

    let App = Router::new()
        .route("/submit", post(SubmitTicket))
        .with_state(SharedState);

    let Listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    println!("Matchmaker running on port 3000");
    axum::serve(Listener, App).await.unwrap();
}

#[allow(non_snake_case)]
async fn SubmitTicket(
    State(Data) : State<Arc<AppState>>,
    Json(Members) : Json<Vec<PartyMember>>
) -> Json<Uuid> {
    let TicketId : Uuid = Uuid::new_v4();
    
    let AvgRating : f64 = Members.iter().map(|M| M.MatchmakingRating).sum::<f64>() / Members.len() as f64;
    
    let NewTicket = MatchmakingTicket {
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

    Data.Tickets.lock().unwrap().push(NewTicket);
    Json(TicketId)
}

#[allow(non_snake_case)]
async fn SendMatchToRoblox(MatchId: Uuid, Members: Vec<PartyMember>) {
    // === CONFIGURATION ===
    let UniverseId = "YOUR_UNIVERSE_ID"; 
    let ApiKey = "YOUR_API_KEY";         
    let Topic = "GlobalMatchmaking";          
    // =====================

    let Client = reqwest::Client::new();
    let PlayerIds : Vec<u64> = Members.iter().map(|M| M.PlayerId).collect();

    let Payload = json!({
        "message": json!({
            "MatchId": MatchId,
            "PlayerIds": PlayerIds
        }).to_string()
    });

    let Url = format!("https://apis.roblox.com/messaging-service/v1/universes/{}/topics/{}/publish", UniverseId, Topic);

    let Response = Client.post(&Url)
        .header("x-api-key", ApiKey)
        .header("Content-Type", "application/json")
        .json(&Payload)
        .send()
        .await;

    match Response {
        Ok(Res) => {
            if !Res.status().is_success() {
                println!("Error sending to Roblox: {:?}", Res.status());
            } else {
                println!("Sent match {} to Roblox", MatchId);
            }
        },
        Err(E) => println!("Network error: {}", E),
    }
}