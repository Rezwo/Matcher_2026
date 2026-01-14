#![allow(non_snake_case)]

use crate::Types::{MatchmakingConfiguration, Region};
use std::collections::HashMap;

pub fn GetStandardConfiguration() -> MatchmakingConfiguration {
    let mut ProximityMap: HashMap<Region, Vec<Region>> = HashMap::new();
    ProximityMap.insert("NorthAmerica".to_string(), vec!["SouthAmerica".to_string()]);
    ProximityMap.insert("Europe".to_string(), vec!["Asia".to_string()]);

    MatchmakingConfiguration {
        MinimumPlayersPerMatch: 1,
        MaximumPlayersPerMatch: 12,
        MaximumPartySize: 4,
        InitialMatchmakingRatingRange: 100.0,
        MatchmakingRatingExpansionPerLevel: 50.0,
        MaximumMatchmakingRatingRange: 500.0,
        RegionProximityMap: ProximityMap,
        MatchmakerTickRateSeconds: 5,
        SearchExpansionIntervalSeconds: 30,
        RobloxUniverseId: "9504219975".to_string(),
        RobloxApiKey: "JqjONCMPHUy3jzH+Sy29t8cXgqyNNedJyvAFODthyZHilIfoZXlKaGJHY2lPaUpTVXpJMU5pSXNJbXRwWkNJNkluTnBaeTB5TURJeExUQTNMVEV6VkRFNE9qVXhPalE1V2lJc0luUjVjQ0k2SWtwWFZDSjkuZXlKaGRXUWlPaUpTYjJKc2IzaEpiblJsY201aGJDSXNJbWx6Y3lJNklrTnNiM1ZrUVhWMGFHVnVkR2xqWVhScGIyNVRaWEoyYVdObElpd2lZbUZ6WlVGd2FVdGxlU0k2SWtweGFrOU9RMDFRU0ZWNU0ycDZTQ3RUZVRJNWREaGpXR2R4ZVU1T1pXUktlWFpCUms5RWRHaDVXa2hwYkVsbWJ5SXNJbTkzYm1WeVNXUWlPaUl4TWpFMk1EZzBOakFpTENKbGVIQWlPakUzTmpnME1UVTJPVGNzSW1saGRDSTZNVGMyT0RReE1qQTVOeXdpYm1KbUlqb3hOelk0TkRFeU1EazNmUS5JcExWZFBnajNtYi15MTBGZUZWX3lGNEExUFVxYWJzaEJaRXJxVmZoZi1oWjFYbGpZMjVGd2dFdktOTkxaeVpuX0cyZTg3QmN6TDhhM1lCSHZvVUFYSklVN1NkYThMR0hyZW9PRFhtWVV0a1NKd3o4UXRubWRrTzV4elFUUkdKN043RXExaURCYmhkRkhUSWR3M2xDRFFWM0x4YXZyUVd5X0RoaFFiWXlWS1BJTlFLNzF5OUNLX1ZidEtiZnpBSHV1YmM1dlVnc3BidzJCQ3hRWFZCbDF2RDhJVHlYSGpFX3RnTVE4UlF0Z1p4SWtkLTh6TUpoNzB4TmhrRVdFZFhsd045VzNJUnVjdzUtaHprSVMxOVZsemJBYjlDRHVxZ1piNWVvd0JJTGlmUE5tRjRfRVY4Q05YZnViS3k2M3hkQXFBTUI1bG9JbVlxRnhBRVY1bzQ0YXc=".to_string(),
        RobloxTopic: "GlobalMatchmaking".to_string(),
        ServerAuthSecret: "MySuperSecretAuthKey123".to_string(),
    }
}
