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

/// Validates configuration values and returns an error message if invalid
pub fn ValidateConfiguration(Config: &MatchmakingConfiguration) -> Result<(), String> {
    // Critical: Division by zero prevention
    if Config.RatingBucketSize <= 0.0 {
        return Err("RATING_BUCKET_SIZE must be greater than 0".to_string());
    }

    // Player count validation
    if Config.MinimumPlayersPerMatch == 0 {
        return Err("MIN_PLAYERS_PER_MATCH must be at least 1".to_string());
    }
    if Config.MinimumPlayersPerMatch > Config.MaximumPlayersPerMatch {
        return Err("MIN_PLAYERS_PER_MATCH must be <= MAX_PLAYERS_PER_MATCH".to_string());
    }
    if Config.MaximumPartySize > Config.MaximumPlayersPerMatch {
        return Err("MAX_PARTY_SIZE must be <= MAX_PLAYERS_PER_MATCH".to_string());
    }

    // Rating validation
    if Config.MinimumRating > Config.MaximumRating {
        return Err("MIN_RATING must be <= MAX_RATING".to_string());
    }
    if Config.InitialMatchmakingRatingRange <= 0.0 {
        return Err("INITIAL_RATING_RANGE must be > 0".to_string());
    }
    if Config.MatchmakingRatingExpansionPerLevel < 0.0 {
        return Err("RATING_EXPANSION_PER_LEVEL must be >= 0".to_string());
    }
    if Config.MaximumMatchmakingRatingRange < Config.InitialMatchmakingRatingRange {
        return Err("MAX_RATING_RANGE must be >= INITIAL_RATING_RANGE".to_string());
    }

    // Time validation
    if Config.SearchExpansionIntervalSeconds <= 0 {
        return Err("SEARCH_EXPANSION_INTERVAL_SECONDS must be > 0".to_string());
    }
    if Config.MatchmakerTickRateSeconds == 0 {
        return Err("MATCHMAKER_TICK_RATE_SECONDS must be > 0".to_string());
    }
    if Config.TicketTtlSeconds <= 0 {
        return Err("TICKET_TTL_SECONDS must be > 0".to_string());
    }

    // Region validation
    if Config.ValidRegions.is_empty() {
        return Err("ValidRegions cannot be empty".to_string());
    }
    if !Config.ValidRegions.contains(&Config.DefaultRegion) {
        return Err("DEFAULT_REGION must be in ValidRegions".to_string());
    }

    // Secret validation
    if Config.ServerAuthSecret.is_empty() {
        return Err("SERVER_AUTH_SECRET cannot be empty".to_string());
    }
    if Config.RobloxApiKey.is_empty() {
        return Err("ROBLOX_API_KEY cannot be empty".to_string());
    }
    if Config.RobloxUniverseId.is_empty() {
        return Err("ROBLOX_UNIVERSE_ID cannot be empty".to_string());
    }

    Ok(())
}

pub fn GetStandardConfiguration() -> MatchmakingConfiguration {
    let mut ProximityMap: HashMap<Region, Vec<Region>> = HashMap::new();
    ProximityMap.insert("NorthAmerica".to_string(), vec!["SouthAmerica".to_string()]);
    ProximityMap.insert("SouthAmerica".to_string(), vec!["NorthAmerica".to_string()]);
    ProximityMap.insert("Europe".to_string(), vec!["Asia".to_string()]);
    ProximityMap.insert("Asia".to_string(), vec!["Europe".to_string()]);
    ProximityMap.insert("Oceania".to_string(), vec!["Asia".to_string()]);

    let mut ValidRegions: HashSet<String> = HashSet::new();
    ValidRegions.insert("NorthAmerica".to_string());
    ValidRegions.insert("SouthAmerica".to_string());
    ValidRegions.insert("Europe".to_string());
    ValidRegions.insert("Asia".to_string());
    ValidRegions.insert("Oceania".to_string());

    let Config = MatchmakingConfiguration {
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
        DebugMode: GetEnvOrDefault("DEBUG", "false")
            .parse()
            .unwrap_or(false),
        HttpTimeoutSeconds: GetEnvOrDefault("HTTP_TIMEOUT_SECONDS", "30")
            .parse()
            .expect("HTTP_TIMEOUT_SECONDS must be a valid u64"),
        RedisPoolMaxSize: GetEnvOrDefault("REDIS_POOL_MAX_SIZE", "16")
            .parse()
            .expect("REDIS_POOL_MAX_SIZE must be a valid u32"),
        RedisPoolMinIdle: GetEnvOrDefault("REDIS_POOL_MIN_IDLE", "4")
            .parse()
            .expect("REDIS_POOL_MIN_IDLE must be a valid u32"),
        SubmitRateLimitPerSecond: GetEnvOrDefault("SUBMIT_RATE_LIMIT", "1000")
            .parse()
            .expect("SUBMIT_RATE_LIMIT must be a valid u32"),
        RegionalRateLimitPerSecond: GetEnvOrDefault("REGIONAL_RATE_LIMIT", "500")
            .parse()
            .expect("REGIONAL_RATE_LIMIT must be a valid u32"),
    };

    // Validate configuration on startup
    if let Err(Error) = ValidateConfiguration(&Config) {
        panic!("Invalid configuration: {}", Error);
    }

    Config
}
