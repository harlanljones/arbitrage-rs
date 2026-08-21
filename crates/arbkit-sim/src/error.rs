//! Errors returned by the paper trading and backtesting simulator.

use arbkit_core::Cents;
use thiserror::Error;

/// Errors arising during simulation and latency modeling.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SimError {
    /// The number of legs in an order or signal is outside the allowed range.
    #[error("invalid leg count: {0} (must be between 2 and {max})", max = arbkit_core::MAX_LEGS)]
    InvalidLegCount(usize),

    /// A leg was requested with non-positive stake.
    #[error("zero or negative stake requested for leg slot {0}")]
    ZeroStake(usize),

    /// Event timestamp regressed in time.
    #[error("timestamp regression: event time {event_ns} ns is before current simulation time {current_ns} ns")]
    TimestampRegression {
        /// The event timestamp.
        event_ns: u64,
        /// The current simulation timestamp.
        current_ns: u64,
    },

    /// The requested venue has no latency profile configured.
    #[error("venue {0} has no latency profile configured")]
    VenueNotConfigured(u16),

    /// Order book is missing or unpopulated for a leg.
    #[error("missing book for venue {venue_id}, outcome {outcome_id}")]
    MissingBook {
        /// The venue ID.
        venue_id: u16,
        /// The outcome ID.
        outcome_id: u32,
    },
}

/// Shorthand result type for simulation operations.
pub type Result<T> = core::result::Result<T, SimError>;

/// Errors constructing a [`crate::bankroll::Bankroll`].
///
/// Per the workspace rule that `Ok(None)`/no-signal is the common case and
/// errors are reserved for malformed input: an insufficient balance at
/// runtime is *not* an error here (see `Bankroll::reserve`, which returns
/// `bool`) — these variants exist only to reject nonsensical construction
/// arguments up front instead of silently truncating or clamping them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum BankrollError {
    /// More initial balances were supplied than a `Bankroll` can track.
    #[error(
        "too many venues: {0} exceeds max of {max}",
        max = crate::bankroll::MAX_BANKROLL_VENUES
    )]
    TooManyVenues(usize),

    /// An initial balance was negative.
    #[error("negative initial balance for venue index {venue}: {cents} cents")]
    NegativeInitialBalance {
        /// Index into the `initial_per_venue` slice passed to `Bankroll::new`.
        venue: usize,
        /// The offending (negative) balance, in cents.
        cents: Cents,
    },
}
