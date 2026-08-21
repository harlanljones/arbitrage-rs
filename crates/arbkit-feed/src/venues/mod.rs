//! Venue-specific message decoders and protocol normalizers.

use arbkit_core::VenueId;

pub mod kalshi;
pub mod polymarket;

pub use kalshi::KalshiParser;
pub use polymarket::PolymarketParser;

/// Unknown or unassigned venue identifier.
pub const VENUE_UNKNOWN: VenueId = 0;

/// Kalshi prediction market exchange identifier.
pub const VENUE_KALSHI: VenueId = 1;

/// Polymarket CLOB exchange identifier.
pub const VENUE_POLYMARKET: VenueId = 2;

/// Betfair Exchange identifier.
pub const VENUE_BETFAIR: VenueId = 3;
