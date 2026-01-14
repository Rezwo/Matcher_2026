#![allow(dead_code)]
#![allow(non_snake_case)]

use serde::{Serialize, Deserialize};
use uuid::Uuid;
use std::collections::HashMap;

pub type PlayerId = u64;
pub type MatchmakingRating = f64;
pub type Region = String; 

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PartyMember {
    pub PlayerId : PlayerId,
    pub PlayerName : String,
    pub MatchmakingRating : MatchmakingRating,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MatchmakingTicket {
    pub TicketId : Uuid,
    pub Members : Vec<PartyMember>,
    pub AverageMatchmakingRating : MatchmakingRating,
    pub PreferredRegion : Region,
    pub AllowedRegions : Vec<Region>,
    pub SubmittedTimestamp : i64,
    pub SearchExpansionLevel : u32,
    pub MinimumMatchmakingRating : MatchmakingRating,
    pub MaximumMatchmakingRating : MatchmakingRating,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MatchmakingConfiguration {
    pub MinimumPlayersPerMatch : u32,
    pub MaximumPlayersPerMatch : u32,
    pub MaximumPartySize : u32,
    pub InitialMatchmakingRatingRange : f64,
    pub MatchmakingRatingExpansionPerLevel : f64,
    pub MaximumMatchmakingRatingRange : f64,
    pub RegionProximityMap : HashMap<Region, Vec<Region>>,
    pub MatchmakerTickRateSeconds : u64,
    pub SearchExpansionIntervalSeconds : i64,
}