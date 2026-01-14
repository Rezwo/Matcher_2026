#![allow(dead_code)]
#![allow(non_snake_case)]

use crate::Types::{MatchmakingConfiguration, Region};
use std::collections::HashMap;

pub fn GetStandardConfiguration() -> MatchmakingConfiguration {
    let mut ProximityMap : HashMap<Region, Vec<Region>> = HashMap::new();
    ProximityMap.insert("NorthAmerica".to_string(), vec!["SouthAmerica".to_string()]);
    ProximityMap.insert("Europe".to_string(), vec!["Asia".to_string()]);

    MatchmakingConfiguration {
        MinimumPlayersPerMatch : 1, 
        MaximumPlayersPerMatch : 12,
        MaximumPartySize : 4,
        InitialMatchmakingRatingRange : 100.0,
        MatchmakingRatingExpansionPerLevel : 50.0,
        MaximumMatchmakingRatingRange : 500.0,
        RegionProximityMap : ProximityMap,
        MatchmakerTickRateSeconds : 5,
        SearchExpansionIntervalSeconds : 30,
    }
}