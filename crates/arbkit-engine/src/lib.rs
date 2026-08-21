//! Arbitrage hot loop, preallocated order book slab, lock-free ring buffer, and latency measurement.
//!
//! `arbkit-engine` is the low-latency core execution engine of `arbkit`. It is
//! designed to meet a strict budget of **p99 < 50 µs** from feed ingestion to signal emission.
//!
//! # Architecture & Hot Path Invariants
//!
//! - **No allocations:** The hot loop executes across a preallocated flat slab of [`arbkit_core::OutcomeBook`]s
//!   and fixed-size event structs.
//! - **No locks:** Communication across thread boundaries uses a pure-Rust, cacheline-padded lock-free SPSC ring buffer.
//! - **No async:** Tokio and network polling terminate at the feed boundary; the engine runs as a synchronous thread.
//! - **No strings:** Identifiers are interned to integers (`MarketId`, `OutcomeId`, `VenueId`).
//! - **Fixed-point arithmetic:** Prices and payouts are strictly integer arithmetic.
//!
//! # Components
//!
//! - [`ring`]: Lock-free SPSC ring buffer for inter-thread handoffs.
//! - [`slab`]: Preallocated flat memory slab indexed by `(MarketId, OutcomeId, VenueId)`.
//! - [`aggregator`]: Fast quote aggregator that evaluates markets with [`arbkit_core::detect`].
//! - [`histogram`]: Fixed-capacity sub-microsecond latency histogram.
//! - [`engine`]: Dedicated single-threaded execution loop.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod aggregator;
pub mod engine;
pub mod error;
pub mod event;
pub mod histogram;
pub mod ring;
pub mod slab;

pub use aggregator::Aggregator;
pub use engine::{Engine, EngineStats};
pub use error::{EngineError, Result};
pub use event::{FeedEventSlot, SignalEvent, SignalEventSlot};
pub use histogram::{LatencyHistogram, LatencySummary, NUM_BINS};
pub use ring::{spsc_ring, AtomicSlot, CachePadded, Consumer, Producer, RingBuffer, RingSlot};
pub use slab::{EngineSlab, MarketConfig, DEFAULT_MAX_MARKETS, MAX_OUTCOMES, MAX_VENUES};
