#![allow(non_snake_case)]

use crate::Types::{MatchmakingConfiguration, Region};
use std::collections::{HashMap, HashSet};
use std::env;

fn GetEnvOrDefault(Key: &str, Default: &str) -> String {
    env::var(Key).unwrap_or_else(|_| Default.to_string())
}

fn GetEnvOrPanic(Key: &str) -> String {
    env::var(Key).unwrap_or_else(|_| panic!("{} environment variable is required", Key))
}

pub fn GetStandardConfiguration() -> MatchmakingConfiguration {
    let mut ProximityMap: HashMap<Region, Vec<Region>> = HashMap::new();
    ProximityMap.insert("NorthAmerica".to_string(), vec!["SouthAmerica".to_string()]);
    ProximityMap.insert("SouthAmerica".to_string(), vec!["NorthAmerica".to_string()]);
    ProximityMap.insert("Europe".to_string(), vec!["Asia".to_string()]);
    ProximityMap.insert("Asia".to_string(), vec!["Europe".to_string()]);

    let mut ValidRegions: HashSet<String> = HashSet::new();
    ValidRegions.insert("NorthAmerica".to_string());
    ValidRegions.insert("SouthAmerica".to_string());
    ValidRegions.insert("Europe".to_string());
    ValidRegions.insert("Asia".to_string());

    MatchmakingConfiguration {
        MinimumPlayersPerMatch: GetEnvOrDefault("MIN_PLAYERS_PER_MATCH", "1")
            .parse()
            .expect("MIN_PLAYERS_PER_MATCH must be a valid u32"),
        MaximumPlayersPerMatch: GetEnvOrDefault("MAX_PLAYERS_PER_MATCH", "12")
            .parse()
            .expect("MAX_PLAYERS_PER_MATCH must be a valid u32"),
        MaximumPartySize: GetEnvOrDefault("MAX_PARTY_SIZE", "4")
            .parse()
            .expect("MAX_PARTY_SIZE must be a valid u32"),
        InitialMatchmakingRatingRange: GetEnvOrDefault("INITIAL_RATING_RANGE", "100.0")
            .parse()
            .expect("INITIAL_RATING_RANGE must be a valid f64"),
        MatchmakingRatingExpansionPerLevel: GetEnvOrDefault("RATING_EXPANSION_PER_LEVEL", "50.0")
            .parse()
            .expect("RATING_EXPANSION_PER_LEVEL must be a valid f64"),
        MaximumMatchmakingRatingRange: GetEnvOrDefault("MAX_RATING_RANGE", "500.0")
            .parse()
            .expect("MAX_RATING_RANGE must be a valid f64"),
        RegionProximityMap: ProximityMap,
        MatchmakerTickRateSeconds: GetEnvOrDefault("MATCHMAKER_TICK_RATE_SECONDS", "5")
            .parse()
            .expect("MATCHMAKER_TICK_RATE_SECONDS must be a valid u64"),
        SearchExpansionIntervalSeconds: GetEnvOrDefault("SEARCH_EXPANSION_INTERVAL_SECONDS", "30")
            .parse()
            .expect("SEARCH_EXPANSION_INTERVAL_SECONDS must be a valid i64"),
        RobloxUniverseId: GetEnvOrPanic("ROBLOX_UNIVERSE_ID"),
        RobloxApiKey: GetEnvOrPanic("ROBLOX_API_KEY"),
        RobloxTopic: GetEnvOrDefault("ROBLOX_TOPIC", "GlobalMatchmaking"),
        ServerAuthSecret: GetEnvOrPanic("SERVER_AUTH_SECRET"),
        MinimumRating: GetEnvOrDefault("MIN_RATING", "0.0")
            .parse()
            .expect("MIN_RATING must be a valid f64"),
        MaximumRating: GetEnvOrDefault("MAX_RATING", "10000.0")
            .parse()
            .expect("MAX_RATING must be a valid f64"),
        DefaultRegion: GetEnvOrDefault("DEFAULT_REGION", "NorthAmerica"),
        ValidRegions,
        TicketTtlSeconds: GetEnvOrDefault("TICKET_TTL_SECONDS", "300")
            .parse()
            .expect("TICKET_TTL_SECONDS must be a valid i64"),
        RatingBucketSize: GetEnvOrDefault("RATING_BUCKET_SIZE", "100.0")
            .parse()
            .expect("RATING_BUCKET_SIZE must be a valid f64"),
    }
}
