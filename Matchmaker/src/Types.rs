#![allow(non_snake_case)]

use serde::{Serialize, Deserialize};
use uuid::Uuid;
use std::collections::{HashMap, HashSet};

pub type PlayerId = u64;
pub type MatchmakingRating = f64;
pub type Region = String;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PartyMember {
    pub PlayerId: PlayerId,
    pub PlayerName: String,
    pub MatchmakingRating: MatchmakingRating,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SubmitTicketRequest {
    pub Members: Vec<PartyMember>,
    pub PreferredRegion: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MatchmakingTicket {
    pub TicketId: Uuid,
    pub Members: Vec<PartyMember>,
    pub AverageMatchmakingRating: MatchmakingRating,
    pub PreferredRegion: Region,
    pub AllowedRegions: HashSet<Region>,
    pub SubmittedTimestamp: i64,
    pub SearchExpansionLevel: u32,
    pub MinimumMatchmakingRating: MatchmakingRating,
    pub MaximumMatchmakingRating: MatchmakingRating,
}

#[derive(Debug, Clone)]
pub struct MatchmakingConfiguration {
    pub MinimumPlayersPerMatch: u32,
    pub MaximumPlayersPerMatch: u32,
    pub MaximumPartySize: u32,
    pub InitialMatchmakingRatingRange: f64,
    pub MatchmakingRatingExpansionPerLevel: f64,
    pub MaximumMatchmakingRatingRange: f64,
    pub RegionProximityMap: HashMap<Region, Vec<Region>>,
    pub MatchmakerTickRateSeconds: u64,
    pub SearchExpansionIntervalSeconds: i64,
    pub RobloxUniverseId: String,
    pub RobloxApiKey: String,
    pub RobloxTopic: String,
    pub ServerAuthSecret: String,
    pub MinimumRating: f64,
    pub MaximumRating: f64,
    pub DefaultRegion: String,
    pub ValidRegions: HashSet<String>,
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub Status: String,
    pub RedisConnected: bool,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub Error: String,
}
