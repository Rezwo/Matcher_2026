#![allow(dead_code)]
#![allow(non_snake_case)]

use crate::Types::{MatchmakingTicket, MatchmakingConfiguration};
use chrono::Utc;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};

pub struct Matchmaker {
    pub Configuration: MatchmakingConfiguration,
}

type BucketId = i64;

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

    pub fn RemoveExpiredTickets(&self, Tickets: &mut Vec<MatchmakingTicket>) -> Vec<MatchmakingTicket> {
        let CurrentTimestamp: i64 = Utc::now().timestamp();
        let TtlSeconds: i64 = self.Configuration.TicketTtlSeconds;

        let (Expired, Active): (Vec<_>, Vec<_>) = Tickets
            .drain(..)
            .partition(|Ticket| CurrentTimestamp - Ticket.SubmittedTimestamp > TtlSeconds);

        *Tickets = Active;
        Expired
    }

    pub fn FindMatches(&self, Tickets: &[MatchmakingTicket]) -> Vec<Vec<MatchmakingTicket>> {
        let TicketCount = Tickets.len();
        if TicketCount == 0 {
            return Vec::new();
        }

        let TicketsByMode = self.GroupByGameMode(Tickets);

        let AllMatches: Vec<Vec<MatchmakingTicket>> = TicketsByMode
            .par_iter()
            .flat_map(|(_ModeName, ModeIndices)| {
                self.FindMatchesForMode(Tickets, ModeIndices)
            })
            .collect();

        AllMatches
    }

    fn GroupByGameMode(&self, Tickets: &[MatchmakingTicket]) -> HashMap<String, Vec<usize>> {
        let mut Groups: HashMap<String, Vec<usize>> = HashMap::new();

        for (Index, Ticket) in Tickets.iter().enumerate() {
            Groups
                .entry(Ticket.GameMode.Name.clone())
                .or_insert_with(Vec::new)
                .push(Index);
        }

        Groups
    }

    fn FindMatchesForMode(
        &self,
        AllTickets: &[MatchmakingTicket],
        ModeIndices: &[usize],
    ) -> Vec<Vec<MatchmakingTicket>> {
        if ModeIndices.is_empty() {
            return Vec::new();
        }

        let FirstTicket = &AllTickets[ModeIndices[0]];
        let MinPlayers = FirstTicket.GameMode.MinPlayers;
        let MaxPlayers = FirstTicket.GameMode.MaxPlayers;

        let BucketSize = self.Configuration.RatingBucketSize;
        let EstimatedMatches = ModeIndices.len() / MinPlayers as usize;
        let mut PotentialMatches: Vec<Vec<MatchmakingTicket>> = Vec::with_capacity(EstimatedMatches.max(1));
        let mut UsedTicketIds: HashSet<uuid::Uuid> = HashSet::with_capacity(ModeIndices.len());

        let ModeTickets: Vec<&MatchmakingTicket> = ModeIndices.iter().map(|&I| &AllTickets[I]).collect();
        let RatingBuckets = self.BuildRatingBucketsForMode(&ModeTickets, BucketSize);

        for (LocalIndex, &GlobalIndex) in ModeIndices.iter().enumerate() {
            let BaseTicket = &AllTickets[GlobalIndex];

            if UsedTicketIds.contains(&BaseTicket.TicketId) {
                continue;
            }

            let mut MatchGroupIndices: Vec<usize> = Vec::with_capacity(MaxPlayers as usize);
            MatchGroupIndices.push(GlobalIndex);
            let mut CurrentPlayerCount: u32 = BaseTicket.Members.len() as u32;

            let CandidateLocalIndices = self.GetCandidatesFromBucketsLocal(
                BaseTicket,
                &RatingBuckets,
                BucketSize,
            );

            for LocalCandidateIndex in CandidateLocalIndices {
                if LocalCandidateIndex <= LocalIndex {
                    continue;
                }

                let GlobalCandidateIndex = ModeIndices[LocalCandidateIndex];
                let CandidateTicket = &AllTickets[GlobalCandidateIndex];

                if UsedTicketIds.contains(&CandidateTicket.TicketId) {
                    continue;
                }

                let CandidateSize: u32 = CandidateTicket.Members.len() as u32;

                if CurrentPlayerCount + CandidateSize > MaxPlayers {
                    continue;
                }

                if self.AreCompatible(BaseTicket, CandidateTicket) && self.HaveRegionOverlap(BaseTicket, CandidateTicket) {
                    MatchGroupIndices.push(GlobalCandidateIndex);
                    CurrentPlayerCount += CandidateSize;

                    if CurrentPlayerCount >= MaxPlayers {
                        break;
                    }
                }
            }

            if CurrentPlayerCount >= MinPlayers {
                for &Idx in &MatchGroupIndices {
                    UsedTicketIds.insert(AllTickets[Idx].TicketId);
                }

                let MatchGroup: Vec<MatchmakingTicket> = MatchGroupIndices
                    .iter()
                    .map(|&Idx| AllTickets[Idx].clone())
                    .collect();

                PotentialMatches.push(MatchGroup);
            }
        }

        PotentialMatches
    }

    fn BuildRatingBuckets(&self, Tickets: &[MatchmakingTicket], BucketSize: f64) -> HashMap<BucketId, Vec<usize>> {
        let EstimatedBuckets = ((self.Configuration.MaximumRating - self.Configuration.MinimumRating) / BucketSize) as usize + 1;
        let mut Buckets: HashMap<BucketId, Vec<usize>> = HashMap::with_capacity(EstimatedBuckets);

        for (Index, Ticket) in Tickets.iter().enumerate() {
            let MinBucket = (Ticket.MinimumMatchmakingRating / BucketSize).floor() as BucketId;
            let MaxBucket = (Ticket.MaximumMatchmakingRating / BucketSize).floor() as BucketId;

            for Bucket in MinBucket..=MaxBucket {
                Buckets.entry(Bucket).or_insert_with(|| Vec::with_capacity(64)).push(Index);
            }
        }

        Buckets
    }

    fn BuildRatingBucketsForMode(&self, Tickets: &[&MatchmakingTicket], BucketSize: f64) -> HashMap<BucketId, Vec<usize>> {
        let mut Buckets: HashMap<BucketId, Vec<usize>> = HashMap::new();

        for (LocalIndex, Ticket) in Tickets.iter().enumerate() {
            let MinBucket = (Ticket.MinimumMatchmakingRating / BucketSize).floor() as BucketId;
            let MaxBucket = (Ticket.MaximumMatchmakingRating / BucketSize).floor() as BucketId;

            for Bucket in MinBucket..=MaxBucket {
                Buckets.entry(Bucket).or_insert_with(|| Vec::with_capacity(64)).push(LocalIndex);
            }
        }

        Buckets
    }

    fn GetCandidatesFromBucketsLocal(
        &self,
        BaseTicket: &MatchmakingTicket,
        Buckets: &HashMap<BucketId, Vec<usize>>,
        BucketSize: f64,
    ) -> Vec<usize> {
        let MinBucket = (BaseTicket.MinimumMatchmakingRating / BucketSize).floor() as BucketId;
        let MaxBucket = (BaseTicket.MaximumMatchmakingRating / BucketSize).floor() as BucketId;

        let mut Candidates: Vec<usize> = Vec::with_capacity(128);
        let mut SeenIndices: HashSet<usize> = HashSet::with_capacity(128);

        for Bucket in MinBucket..=MaxBucket {
            if let Some(Indices) = Buckets.get(&Bucket) {
                for &Idx in Indices {
                    if !SeenIndices.contains(&Idx) {
                        SeenIndices.insert(Idx);
                        Candidates.push(Idx);
                    }
                }
            }
        }

        Candidates
    }

    fn GetCandidatesFromBuckets(
        &self,
        BaseTicket: &MatchmakingTicket,
        Buckets: &HashMap<BucketId, Vec<usize>>,
        BucketSize: f64,
    ) -> Vec<usize> {
        let MinBucket = (BaseTicket.MinimumMatchmakingRating / BucketSize).floor() as BucketId;
        let MaxBucket = (BaseTicket.MaximumMatchmakingRating / BucketSize).floor() as BucketId;

        let mut Candidates: Vec<usize> = Vec::with_capacity(128);
        let mut SeenIndices: HashSet<usize> = HashSet::with_capacity(128);

        for Bucket in MinBucket..=MaxBucket {
            if let Some(Indices) = Buckets.get(&Bucket) {
                for &Idx in Indices {
                    if !SeenIndices.contains(&Idx) {
                        SeenIndices.insert(Idx);
                        Candidates.push(Idx);
                    }
                }
            }
        }

        Candidates
    }

    pub fn GetMatchedTicketIds(&self, Matches: &[Vec<MatchmakingTicket>]) -> HashSet<uuid::Uuid> {
        let TotalTickets: usize = Matches.iter().map(|G| G.len()).sum();
        let mut Ids: HashSet<uuid::Uuid> = HashSet::with_capacity(TotalTickets);
        for Group in Matches {
            for Ticket in Group {
                Ids.insert(Ticket.TicketId);
            }
        }
        Ids
    }

    #[inline]
    fn AreCompatible(&self, First: &MatchmakingTicket, Second: &MatchmakingTicket) -> bool {
        let MaxOfMins: f64 = First.MinimumMatchmakingRating.max(Second.MinimumMatchmakingRating);
        let MinOfMaxs: f64 = First.MaximumMatchmakingRating.min(Second.MaximumMatchmakingRating);
        MaxOfMins <= MinOfMaxs
    }

    #[inline]
    fn HaveRegionOverlap(&self, First: &MatchmakingTicket, Second: &MatchmakingTicket) -> bool {
        !First.AllowedRegions.is_disjoint(&Second.AllowedRegions)
    }
}
