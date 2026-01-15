#![allow(non_snake_case)]

use serde::{Serialize, Deserialize};
use smallvec::SmallVec;
use uuid::Uuid;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicU64;

pub type PlayerId = u64;
pub type MatchmakingRating = f64;
pub type Region = String;
pub type PartyMembers = SmallVec<[PartyMember; 4]>;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
pub struct GameMode {
    pub Name: String,
    pub MinPlayers: u32,
    pub MaxPlayers: u32,
}

impl Default for GameMode {
    fn default() -> Self {
        Self {
            Name: "FreeForAll".to_string(),
            MinPlayers: 1,
            MaxPlayers: 12,
        }
    }
}

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
    pub GameMode: Option<GameMode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TicketStatus {
    Queued,
    Matched,
}

impl Default for TicketStatus {
    fn default() -> Self {
        TicketStatus::Queued
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MatchmakingTicket {
    pub TicketId: Uuid,
    pub Members: PartyMembers,
    pub AverageMatchmakingRating: MatchmakingRating,
    pub PreferredRegion: Region,
    pub AllowedRegions: HashSet<Region>,
    pub SubmittedTimestamp: i64,
    pub SearchExpansionLevel: u32,
    pub MinimumMatchmakingRating: MatchmakingRating,
    pub MaximumMatchmakingRating: MatchmakingRating,
    #[serde(default)]
    pub Status: TicketStatus,
    #[serde(default)]
    pub GameMode: GameMode,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TicketIndexEntry {
    pub TicketId: Uuid,
    pub MinRating: f64,
    pub MaxRating: f64,
    pub Timestamp: i64,
    pub PlayerCount: u32,
    pub Regions: SmallVec<[String; 2]>,
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
    pub TicketTtlSeconds: i64,
    pub RatingBucketSize: f64,
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

#[derive(Debug, Serialize)]
pub struct TicketStatusResponse {
    pub TicketId: Uuid,
    pub Status: String,
    pub QueuePosition: Option<usize>,
    pub WaitTimeSeconds: i64,
    pub ExpansionLevel: u32,
    pub EstimatedWaitSeconds: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct MetricsSnapshot {
    pub QueueSize: u64,
    pub TotalTicketsProcessed: u64,
    pub TotalMatchesCreated: u64,
    pub TotalTicketsExpired: u64,
    pub AverageWaitTimeSeconds: f64,
    pub MatchesLastMinute: u64,
}

pub struct MatchmakerMetrics {
    pub TotalTicketsProcessed: AtomicU64,
    pub TotalMatchesCreated: AtomicU64,
    pub TotalTicketsExpired: AtomicU64,
    pub TotalWaitTimeSeconds: AtomicU64,
    pub MatchedTicketCount: AtomicU64,
}

impl MatchmakerMetrics {
    pub fn new() -> Self {
        Self {
            TotalTicketsProcessed: AtomicU64::new(0),
            TotalMatchesCreated: AtomicU64::new(0),
            TotalTicketsExpired: AtomicU64::new(0),
            TotalWaitTimeSeconds: AtomicU64::new(0),
            MatchedTicketCount: AtomicU64::new(0),
        }
    }

    pub fn GetAverageWaitTime(&self) -> f64 {
        let TotalWait = self.TotalWaitTimeSeconds.load(std::sync::atomic::Ordering::Relaxed);
        let MatchedCount = self.MatchedTicketCount.load(std::sync::atomic::Ordering::Relaxed);
        if MatchedCount == 0 {
            0.0
        } else {
            TotalWait as f64 / MatchedCount as f64
        }
    }
}

impl Default for MatchmakerMetrics {
    fn default() -> Self {
        Self::new()
    }
}
