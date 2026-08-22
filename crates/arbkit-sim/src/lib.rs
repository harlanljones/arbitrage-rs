//! Paper trading simulation, latency modeling, and phantom arbitrage measurement.
//!
//! An arbitrage detector operating on live feeds identifies opportunities that
//! look profitable in snapshot data, but many cannot be monetized when executed
//! over real network connections against real matching engines.
//!
//! This crate provides:
//!
//! - **Paper Trading Simulator & Backtester:** Replays historical order book states
//!   or simulates live execution against resting liquidity.
//! - **Latency Modeling:** Computes wire transit delays, venue matching engine latencies,
//!   and queue front-running degradation on a per-venue basis.
//! - **Phantom Arbitrage Measurement:** Quantifies the "phantom rate" — the percentage
//!   of detected arbitrage signals that were unfillable or decayed before execution.
//! - **PnL & Fill Accounting:** Accurately accounts for realized profits, fill ratios,
//!   slippage, and venue fees with strict pessimistic integer rounding.
//!
//! # Hot Path Safety
//!
//! In keeping with the workspace-wide rules:
//! - All accounting uses whole cents ([`arbkit_core::Cents`]) and fixed-point arithmetic.
//! - No floating-point operations exist in any execution decision or PnL accounting path.
//! - No dynamic allocations occur during individual leg execution evaluation.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod accounting;
pub mod bankroll;
pub mod error;
pub mod latency;
pub mod order;
pub mod phantom;
pub mod simulator;

/// The widest execution a simulation accepts, in legs.
///
/// `detect_book` plans stake across up to [`MAX_CHUNKS`] `(leg, level)`
/// chunks, and the pipeline replays every chunk as its own simulated leg.
/// Capping the simulator at the old four-outcome width would silently
/// reject — or worse, callers would silently skip — exactly the multi-chunk
/// plans that depth-aware detection exists to produce.
pub const MAX_SIM_LEGS: usize = arbkit_core::arb::MAX_CHUNKS;

pub use accounting::{ExecutionPnl, SimulationStats};
pub use bankroll::{Bankroll, MAX_BANKROLL_VENUES};
pub use error::{BankrollError, Result, SimError};
pub use latency::{LatencyModel, LatencyProfile, MAX_CONFIGURED_VENUES};
pub use order::{
    LegFillResult, LegFillStatus, PartialFillReason, SimulatedLegOrder, UnfilledReason,
};
pub use phantom::{ArbExecutionClassification, PhantomReason, PhantomStats};
pub use simulator::{ChasePolicy, ExecutionReport, SimConfig, Simulator};
