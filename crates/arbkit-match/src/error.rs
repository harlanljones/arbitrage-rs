//! Errors raised by event matching, string interning, and market alignment.

use arbkit_core::{Line, MarketId, MarketKind, OutcomeId, VenueId};
use thiserror::Error;

use crate::alignment::OutcomeSide;

/// Errors arising from market matching, normalization, and registry lookups.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MatchError {
    /// A venue was not recognized or registered.
    #[error("unknown venue id: {0}")]
    UnknownVenueId(VenueId),

    /// A venue name string was not found in the registry.
    #[error("unknown venue name: {0}")]
    UnknownVenueName(String),

    /// A venue-specific symbol was not found in the outcome mapping table.
    #[error("unknown symbol '{1}' for venue {0}")]
    UnknownSymbol(VenueId, String),

    /// A team string could not be matched to any known canonical team.
    #[error("unrecognized team name or code: '{0}'")]
    UnrecognizedTeam(String),

    /// A matchup string could not be parsed into home and away teams.
    #[error("malformed matchup string: '{0}'")]
    MalformedMatchup(String),

    /// A venue-specific ticker string could not be parsed.
    #[error("malformed ticker '{0}': {1}")]
    MalformedTicker(String, &'static str),

    /// Two market kinds were expected to match but differed.
    #[error("market kind mismatch: expected {expected:?}, got {actual:?}")]
    MarketKindMismatch {
        /// The expected market kind.
        expected: MarketKind,
        /// The actual market kind encountered.
        actual: MarketKind,
    },

    /// Two lines were expected to match but differed.
    #[error("line mismatch: expected {expected:?}, got {actual:?}")]
    LineMismatch {
        /// The expected line.
        expected: Line,
        /// The actual line encountered.
        actual: Line,
    },

    /// Both legs backed the same side of a proposition rather than opposite sides.
    #[error("both legs back the same outcome side: {0:?}")]
    SameSide(OutcomeSide),

    /// Two outcome sides are incompatible for binary pairing.
    #[error("incompatible outcome sides: {0:?} and {1:?}")]
    IncompatibleSides(OutcomeSide, OutcomeSide),

    /// Attempted to pair outcomes from two different events.
    #[error("event mismatch: expected event {expected}, got event {actual}")]
    EventMismatch {
        /// The expected canonical event ID.
        expected: u32,
        /// The actual canonical event ID encountered.
        actual: u32,
    },

    /// The requested event ID was not found in the registry.
    #[error("canonical event {0} not found")]
    EventNotFound(u32),

    /// The requested market ID was not found in the registry.
    #[error("canonical market {0} not found")]
    MarketNotFound(MarketId),

    /// The requested outcome ID was not found in the registry.
    #[error("canonical outcome {0} not found")]
    OutcomeNotFound(OutcomeId),

    /// An invalid number of legs was provided for alignment.
    #[error("invalid leg count for alignment: {0}")]
    InvalidLegCount(usize),

    /// The maximum capacity for fixed tables was exceeded.
    #[error("capacity exceeded for table: {0}")]
    CapacityExceeded(&'static str),

    /// A duplicate registration was attempted.
    #[error("duplicate registration: {0}")]
    DuplicateRegistration(String),
}

/// Shorthand for results carrying a [`MatchError`].
pub type Result<T> = core::result::Result<T, MatchError>;
