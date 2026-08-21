//! Canonical event and market registry, team normalization, and zero-allocation hot-path matching.
//!
//! This crate is the alignment core of `arbkit`: it takes disparate venue-specific identifiers
//! (such as Kalshi tickers `"KXNBAGAME-26AUG18BOSLAL"`, Polymarket CLOB tokens, and sportsbook
//! names like `"Boston Celtics vs Los Angeles Lakers"`) and maps them onto canonical [`MarketId`]
//! and [`OutcomeId`] handles.
//!
//! # Architecture and Hot Path Rules
//!
//! 1. **Zero Allocations on the Hot Path**: All string normalization, ticker parsing, and registry
//!    population occurs at startup or at the asynchronous feed ingestion boundary. The engine hot loop
//!    queries preallocated, cache-friendly flat lookup tables ([`HotLookupTable`]) indexed by compact integer IDs.
//! 2. **No Strings on the Hot Path**: Venue names and contract symbols are interned into numeric [`VenueId`]
//!    and [`OutcomeId`] values at the feed boundary using [`VenueRegistry`] and [`StringInterner`].
//! 3. **Strict Proposition Alignment**: Validates that legs across venues represent genuinely opposite
//!    sides of the same proposition (e.g. Lakers -3.5 matches Celtics +3.5 on a Celtics home game,
//!    Over 220.5 matches Under 220.5). Disparate lines or mismatched markets are rejected deterministically.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod alignment;
pub mod error;
pub mod intern;
pub mod kalshi;
pub mod lookup;
pub mod registry;
pub mod team;

pub use alignment::{
    align_moneyline, align_spread, align_total, validate_binary_pair, OutcomeSide,
};
pub use arbkit_core::{Line, MarketId, MarketKind, OutcomeId, VenueId};
pub use error::{MatchError, Result};
pub use intern::{StringInterner, VenueRegistry};
pub use kalshi::{parse_kalshi_ticker, KalshiTicker};
pub use lookup::{HotLookupTable, OutcomeRecord};
pub use registry::{CanonicalEvent, CanonicalMarket, CanonicalOutcome, CanonicalRegistry};
pub use team::{lookup_team, normalize_string, parse_matchup, CanonicalTeam, Matchup, Sport};
