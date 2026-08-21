//! Venue connectors, market message parsing, feed normalization, and binary tape recording.
//!
//! `arbkit-feed` handles ingestion of market data feeds from prediction markets and
//! sportsbooks. Out-of-band network message decoders (Kalshi, Polymarket) convert raw
//! WebSocket payloads into flat, zero-allocation [`FeedEvent`] and [`FeedMessage`]
//! records. A high-performance binary tape format enables deterministic recording and
//! sub-microsecond replay of historical market feeds for backtesting and benchmarking.
//!
//! # Hot-Path Invariants
//!
//! - **Zero allocations during replay and transmission:** [`FeedEvent`] and [`FeedMessage`]
//!   are stack-allocated, `Copy` types that traverse ring buffers into the hot loop without
//!   heap activity.
//! - **Exact integer price conversion:** Quotes (cents or decimal strings) are parsed directly
//!   into parts-per-million [`arbkit_core::Prob`] without reciprocal chain floating-point drift.
//! - **Sequence gap detection:** Feeds track monotonicity; sequence discontinuities
//!   immediately signal that the downstream order book state is stale.
//!
//! # Architecture
//!
//! - [`event`]: Feed event definitions ([`FeedEvent`], [`FeedMessage`], [`TradeSide`]).
//! - [`venues`]: Venue-specific decoders ([`KalshiParser`], [`PolymarketParser`]).
//! - [`tape`]: Binary recording ([`TapeWriter`]), deserialization ([`TapeReader`]),
//!   and deterministic replay stream ([`TapePlayer`]).

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod error;
pub mod event;
pub mod tape;
pub mod venues;

pub use error::{FeedError, Result};
pub use event::{FeedEvent, FeedMessage, TradeSide, MAX_EVENTS_PER_MESSAGE};
pub use tape::{
    TapeHeader, TapePlayer, TapeReader, TapeWriter, TAPE_HEADER_SIZE, TAPE_MAGIC, TAPE_VERSION,
};
pub use venues::{
    kalshi::{kalshi_stake_fee, KalshiParser, KalshiSequenceTracker},
    polymarket::{parse_decimal_prob, parse_decimal_size_cents, PolymarketParser},
    VENUE_BETFAIR, VENUE_KALSHI, VENUE_POLYMARKET, VENUE_UNKNOWN,
};
