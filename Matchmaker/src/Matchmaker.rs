#![allow(dead_code)]
#![allow(non_snake_case)]

use crate::Types::{MatchmakingTicket, MatchmakingConfiguration};
use chrono::Utc;
use std::collections::HashSet;

pub struct Matchmaker {
    pub Configuration: MatchmakingConfiguration,
}

impl Matchmaker {
    pub fn new(CurrentConfiguration: MatchmakingConfiguration) -> Self {
        Self {
            Configuration: CurrentConfiguration,
        }
    }

    pub fn ExpandTickets(&self, Tickets: &mut Vec<MatchmakingTicket>) {
        let CurrentTimestamp: i64 = Utc::now().timestamp();

        for Ticket in Tickets.iter_mut() {
            let TimeInQueue: i64 = CurrentTimestamp.saturating_sub(Ticket.SubmittedTimestamp);
            let ExpansionsNeeded: u32 = (TimeInQueue / self.Configuration.SearchExpansionIntervalSeconds)
                .try_into()
                .unwrap_or(u32::MAX);

            if ExpansionsNeeded > Ticket.SearchExpansionLevel {
                self.ApplyExpansion(Ticket, ExpansionsNeeded);
            }
        }
    }

    fn ApplyExpansion(&self, Ticket: &mut MatchmakingTicket, TargetLevel: u32) {
        Ticket.SearchExpansionLevel = TargetLevel;

        let RatingExpansion: f64 = self.Configuration.MatchmakingRatingExpansionPerLevel * (TargetLevel as f64);
        let TotalRatingRange: f64 = (self.Configuration.InitialMatchmakingRatingRange + RatingExpansion)
            .min(self.Configuration.MaximumMatchmakingRatingRange);

        Ticket.MinimumMatchmakingRating = Ticket.AverageMatchmakingRating - TotalRatingRange;
        Ticket.MaximumMatchmakingRating = Ticket.AverageMatchmakingRating + TotalRatingRange;

        if TargetLevel >= 2 {
            if let Some(NearbyRegions) = self.Configuration.RegionProximityMap.get(&Ticket.PreferredRegion) {
                for Region in NearbyRegions {
                    Ticket.AllowedRegions.insert(Region.clone());
                }
            }
        }
    }

    pub fn FindMatches(&self, Tickets: &[MatchmakingTicket]) -> Vec<Vec<MatchmakingTicket>> {
        let mut PotentialMatches: Vec<Vec<MatchmakingTicket>> = Vec::new();
        let mut UsedTicketIds: HashSet<uuid::Uuid> = HashSet::new();

        for (Index, BaseTicket) in Tickets.iter().enumerate() {
            if UsedTicketIds.contains(&BaseTicket.TicketId) {
                continue;
            }

            let mut MatchGroup: Vec<MatchmakingTicket> = vec![BaseTicket.clone()];
            let mut CurrentPlayerCount: u32 = BaseTicket.Members.len() as u32;
            let mut FoundIds: Vec<uuid::Uuid> = vec![BaseTicket.TicketId];

            for CandidateTicket in Tickets.iter().skip(Index + 1) {
                if UsedTicketIds.contains(&CandidateTicket.TicketId) {
                    continue;
                }

                let CandidateSize: u32 = CandidateTicket.Members.len() as u32;

                if CurrentPlayerCount + CandidateSize > self.Configuration.MaximumPlayersPerMatch {
                    continue;
                }

                if self.AreCompatible(BaseTicket, CandidateTicket) && self.HaveRegionOverlap(BaseTicket, CandidateTicket) {
                    MatchGroup.push(CandidateTicket.clone());
                    CurrentPlayerCount += CandidateSize;
                    FoundIds.push(CandidateTicket.TicketId);

                    if CurrentPlayerCount >= self.Configuration.MinimumPlayersPerMatch {
                        break;
                    }
                }
            }

            if CurrentPlayerCount >= self.Configuration.MinimumPlayersPerMatch {
                for Id in FoundIds {
                    UsedTicketIds.insert(Id);
                }
                PotentialMatches.push(MatchGroup);
            }
        }

        PotentialMatches
    }

    pub fn GetMatchedTicketIds(&self, Matches: &[Vec<MatchmakingTicket>]) -> HashSet<uuid::Uuid> {
        Matches
            .iter()
            .flat_map(|Group| Group.iter().map(|Ticket| Ticket.TicketId))
            .collect()
    }

    fn AreCompatible(&self, First: &MatchmakingTicket, Second: &MatchmakingTicket) -> bool {
        let MaxOfMins: f64 = First.MinimumMatchmakingRating.max(Second.MinimumMatchmakingRating);
        let MinOfMaxs: f64 = First.MaximumMatchmakingRating.min(Second.MaximumMatchmakingRating);
        MaxOfMins <= MinOfMaxs
    }

    fn HaveRegionOverlap(&self, First: &MatchmakingTicket, Second: &MatchmakingTicket) -> bool {
        !First.AllowedRegions.is_disjoint(&Second.AllowedRegions)
    }
}
