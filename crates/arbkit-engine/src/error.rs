//! Errors produced by the arbitrage engine.

use arbkit_core::ArbError;
use thiserror::Error;

/// Errors produced during engine configuration, execution, and ring operations.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum EngineError {
    /// An error originated from core arbitrage calculations.
    #[error("domain core error: {0}")]
    Core(#[from] ArbError),

    /// Market identifier exceeds preallocated engine capacity.
    #[error("market id {market_id} exceeds slab capacity {capacity}")]
    MarketOutOfRange {
        /// The invalid market ID.
        market_id: u32,
        /// Maximum capacity.
        capacity: usize,
    },

    /// Outcome identifier exceeds maximum outcomes per market.
    #[error("outcome id {outcome_id} exceeds maximum outcomes {max_outcomes}")]
    OutcomeOutOfRange {
        /// The invalid outcome ID.
        outcome_id: u32,
        /// Maximum outcomes.
        max_outcomes: usize,
    },

    /// Venue identifier exceeds maximum supported venues.
    #[error("venue id {venue_id} exceeds maximum venues {max_venues}")]
    VenueOutOfRange {
        /// The invalid venue ID.
        venue_id: u16,
        /// Maximum venues.
        max_venues: usize,
    },

    /// The ring buffer was full during an enqueue operation.
    #[error("ring buffer is full")]
    RingBufferFull,
}

/// Result type shorthand for engine operations.
pub type Result<T> = core::result::Result<T, EngineError>;
