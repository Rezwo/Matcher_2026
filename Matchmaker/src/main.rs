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

use crate::Types::{MatchmakingTicket, PartyMember};
use crate::Configurations::GetStandardConfiguration;

struct AppState {
    pub Tickets : Mutex<Vec<MatchmakingTicket>>,
    pub MatchmakerInstance : Matchmaker::Matchmaker,
}

#[tokio::main]
async fn main() {
    let SharedState = Arc::new(AppState {
        Tickets : Mutex::new(Vec::new()),
        MatchmakerInstance : Matchmaker::Matchmaker::new(GetStandardConfiguration()),
    });

    let LoopState = SharedState.clone();
    
    tokio::spawn(async move {
        let TickRate = LoopState.MatchmakerInstance.Configuration.MatchmakerTickRateSeconds;
        let mut Interval = interval(Duration::from_secs(TickRate));
        
        loop {
            Interval.tick().await;
            
            let mut Tickets = LoopState.Tickets.lock().unwrap();
            
            LoopState.MatchmakerInstance.ExpandTickets(&mut Tickets);

            let Matches = LoopState.MatchmakerInstance.FindMatches(Tickets.clone());

            for Match in Matches {
                let MatchId : Uuid = Uuid::new_v4();
                let PlayerCount : usize = Match.iter().map(|T| T.Members.len()).sum();
                
                println!("Match Created: {} with {} players", MatchId, PlayerCount);

                let MatchedIds : Vec<Uuid> = Match.iter().map(|T| T.TicketId).collect();
                
                Tickets.retain(|T| !MatchedIds.contains(&T.TicketId));
                
            }
        }
    });

    let App = Router::new()
        .route("/submit", post(SubmitTicket))
        .with_state(SharedState);

    let Listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    println!("Server running on port 3000");
    axum::serve(Listener, App).await.unwrap();
}

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