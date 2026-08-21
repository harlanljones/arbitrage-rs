//! Zero-allocation hot-path lookup tables.
//!
//! On the trading engine hot path, all lookups are indexed by compact numeric identifiers
//! ([`OutcomeId`], [`MarketId`], [`arbkit_core::VenueId`]). The structures here are preallocated
//! flat arrays/slabs that perform index operations with no locking, no hashing, and zero heap
//! allocation.

use arbkit_core::{MarketId, MarketKind, OutcomeId};

use crate::alignment::OutcomeSide;

/// Fast, `Copy` outcome metadata held in flat preallocated lookup slabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutcomeRecord {
    /// Canonical market identifier.
    pub market_id: MarketId,
    /// Canonical opposite outcome identifier for binary markets, or `None`.
    pub opposite_outcome_id: Option<OutcomeId>,
    /// The market kind and handicap line.
    pub market_kind: MarketKind,
    /// The side or role of this outcome.
    pub side: OutcomeSide,
}

/// Hot-path lookup table indexed directly by [`OutcomeId`].
///
/// Designed to be constructed at system initialization and referenced across the hot engine loop.
#[derive(Debug, Clone)]
pub struct HotLookupTable {
    records: Vec<Option<OutcomeRecord>>,
}

impl HotLookupTable {
    /// Create an empty hot lookup table with preallocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            records: Vec::with_capacity(capacity),
        }
    }

    /// Construct a hot lookup table from a slice of optional outcome records.
    pub fn from_records(records: Vec<Option<OutcomeRecord>>) -> Self {
        Self { records }
    }

    /// Insert or update an outcome record at the given [`OutcomeId`] index.
    pub fn set(&mut self, outcome_id: OutcomeId, record: OutcomeRecord) {
        let idx = outcome_id as usize;
        if idx >= self.records.len() {
            self.records.resize(idx + 1, None);
        }
        self.records[idx] = Some(record);
    }

    /// Retrieve the [`OutcomeRecord`] for an [`OutcomeId`].
    #[inline]
    pub fn get(&self, outcome_id: OutcomeId) -> Option<&OutcomeRecord> {
        self.records
            .get(outcome_id as usize)
            .and_then(|r| r.as_ref())
    }

    /// Lookup the canonical [`MarketId`] for a given [`OutcomeId`].
    #[inline]
    pub fn market_of(&self, outcome_id: OutcomeId) -> Option<MarketId> {
        self.get(outcome_id).map(|r| r.market_id)
    }

    /// Lookup the opposite canonical [`OutcomeId`] for a binary market proposition.
    #[inline]
    pub fn opposite_of(&self, outcome_id: OutcomeId) -> Option<OutcomeId> {
        self.get(outcome_id).and_then(|r| r.opposite_outcome_id)
    }

    /// Lookup the [`MarketKind`] for a given [`OutcomeId`].
    #[inline]
    pub fn kind_of(&self, outcome_id: OutcomeId) -> Option<MarketKind> {
        self.get(outcome_id).map(|r| r.market_kind)
    }

    /// Lookup the [`OutcomeSide`] for a given [`OutcomeId`].
    #[inline]
    pub fn side_of(&self, outcome_id: OutcomeId) -> Option<OutcomeSide> {
        self.get(outcome_id).map(|r| r.side)
    }

    /// Verify whether two outcome IDs represent opposite sides of the same proposition.
    #[inline]
    pub fn is_opposite(&self, a: OutcomeId, b: OutcomeId) -> bool {
        self.opposite_of(a) == Some(b)
    }

    /// Total number of outcome slots allocated.
    #[inline]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether the table contains no outcome records.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arbkit_core::Line;

    #[test]
    fn hot_lookup_table_retrieval_and_opposite_checking() {
        let mut table = HotLookupTable::with_capacity(16);

        let kind = MarketKind::Spread(Line::from_hundredths(-350));
        let outcome_a: OutcomeId = 10;
        let outcome_b: OutcomeId = 11;
        let market_id: MarketId = 42;

        table.set(
            outcome_a,
            OutcomeRecord {
                market_id,
                opposite_outcome_id: Some(outcome_b),
                market_kind: kind,
                side: OutcomeSide::HomeCover,
            },
        );

        table.set(
            outcome_b,
            OutcomeRecord {
                market_id,
                opposite_outcome_id: Some(outcome_a),
                market_kind: kind,
                side: OutcomeSide::AwayCover,
            },
        );

        assert_eq!(table.market_of(outcome_a), Some(market_id));
        assert_eq!(table.market_of(outcome_b), Some(market_id));
        assert_eq!(table.opposite_of(outcome_a), Some(outcome_b));
        assert_eq!(table.opposite_of(outcome_b), Some(outcome_a));
        assert!(table.is_opposite(outcome_a, outcome_b));
        assert!(table.is_opposite(outcome_b, outcome_a));
        assert_eq!(table.kind_of(outcome_a), Some(kind));
        assert_eq!(table.side_of(outcome_a), Some(OutcomeSide::HomeCover));
        assert_eq!(table.side_of(outcome_b), Some(OutcomeSide::AwayCover));

        assert_eq!(table.get(999), None);
        assert!(!table.is_opposite(outcome_a, 999));
    }
}
