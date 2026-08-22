//! Odds arithmetic and cross-venue arbitrage detection, in pure Rust.
//!
//! This crate is the domain core of `arbkit`: it knows what a price is,
//! what an order book looks like, what a venue charges, and when a set of
//! prices across venues adds up to less than certainty. It deliberately
//! depends on nothing but `thiserror` — no tokio, no HTTP, no WebSocket, no
//! clock, no network. `cargo test -p arbkit-core` runs offline in
//! milliseconds and needs no credentials for any venue.
//!
//! Connectors live in `arbkit-feed`, the canonical event registry in
//! `arbkit-match`, and the single-threaded hot loop in `arbkit-engine`.
//!
//! # Conventions
//!
//! - **Prices are integers.** [`Prob`] is parts per million; [`Odds`] is
//!   decimal odds in micro-units. Floating point appears only in constructors
//!   named `_f64`, at the feed boundary, and in `as_f64` accessors used for
//!   display. The reason is in the [`price`] module docs, and it is not
//!   stylistic: float rounding in the reciprocal chain manufactures edges that
//!   were never quoted.
//! - **Money is whole cents**, as `i64` ([`Cents`]). Signed, because profit
//!   can be negative.
//! - **No arbitrage is not an error.** [`detect`] returns `Ok(None)` for every
//!   market condition — no edge, no depth, an edge that stake rounding eats.
//!   [`ArbError`] is reserved for malformed input.
//! - **Costs go in before the comparison, never after.** A [`Fee`] transforms
//!   a quoted price into an effective one, and the arbitrage condition is
//!   evaluated on effective prices only.
//! - **Rounding always favours the pessimistic reading.** Payouts floor,
//!   effective prices ceil, stakes round down to a tradeable increment. Every
//!   number this crate reports is one you should be able to beat, not one you
//!   need to hit exactly.
//!
//! # Example
//!
//! ```
//! use arbkit_core::{detect, Fee, Leg, Prob};
//!
//! # fn main() -> arbkit_core::Result<()> {
//! // 48 cents on one venue, 50 on another: 98 cents to buy a dollar. Both
//! // legs carry their venue's cost and the depth actually resting there.
//! let legs = [
//!     Leg {
//!         venue: 0,
//!         outcome: 0,
//!         quoted: Prob::from_cents(48)?,
//!         fee: Fee::StakeFeeBps(364),
//!         capacity: 120_000,
//!         increment: 48,
//!     },
//!     Leg {
//!         venue: 1,
//!         outcome: 1,
//!         quoted: Prob::from_cents(50)?,
//!         fee: Fee::CommissionBps(200),
//!         capacity: 500_000,
//!         increment: 1,
//!     },
//! ];
//!
//! // A 200 bp raw edge, against a 364 bp stake fee on one side and 200 bp of
//! // commission on the other. There was never anything here.
//! assert_eq!(detect(&legs, 100_000)?, None);
//!
//! // The same prices with no fees clear a real, if thin, profit — the 204 bp
//! // the raw prices imply, kept intact despite the 48-cent contract size on
//! // one leg. Sizing that leg to 1_020 contracts (48_960 cents) buys exactly
//! // 102_000 cents of payout, and 51_000 on the other side matches it to the
//! // cent, so the payouts stay equal and 99_960 staked returns 2_040 either
//! // way. Rounding the equal-payoff division down to the increment, as this
//! // crate used to, staked 48_960 and 51_020 and cleared only 2_020.
//! let free: Vec<Leg> = legs.iter().map(|leg| Leg { fee: Fee::None, ..*leg }).collect();
//! let signal = detect(&free, 100_000)?.expect("98c for a dollar is an arbitrage");
//! assert_eq!(signal.profit_bps, 204);
//! # Ok(())
//! # }
//! ```
//!
//! # What this crate cannot check
//!
//! [`detect`] takes it on faith that the legs handed to it are mutually
//! exclusive, collectively exhaustive sides of the *same* market. Prices drawn
//! from two different games sum to whatever they like and look exactly like a
//! large edge. Establishing that two venues are quoting the same thing is the
//! matcher's problem, and it is a harder one than anything in here.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod arb;
pub mod book;
pub mod error;
pub mod fee;
pub mod fill;
pub mod market;
pub mod price;

#[allow(deprecated)]
pub use arb::{detect, frictionless_leg, Allocation, Leg, Signal, MAX_LEGS};
pub use book::{Cents, Level, MarketId, OutcomeBook, OutcomeId, VenueId, MAX_LEVELS};
pub use error::{ArbError, Result};
pub use fee::{kalshi_stake_fee_bps, Fee};
pub use fill::DepthDiscount;
pub use market::{Line, MarketKind};
pub use price::{Odds, Prob, ODDS_ONE, PPM};
