//! Errors raised by the domain core.

use thiserror::Error;

/// Something the caller got wrong.
///
/// Every variant here is a *misconfiguration* — a price outside the range a
/// price can occupy, a market with fewer than two outcomes, a stake increment
/// of zero. None of them describe a market condition. "There is no arbitrage
/// right now" is the overwhelmingly common case and is reported as `None`, not
/// as an error; see [`crate::arb::detect`].
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ArbError {
    /// A probability outside `1..=1_000_000` parts per million.
    ///
    /// Zero is excluded deliberately: a zero-probability outcome implies
    /// infinite decimal odds, which no venue quotes and which would divide by
    /// zero everywhere downstream.
    #[error("implied probability {0} ppm is outside 1..=1_000_000")]
    ProbOutOfRange(u32),

    /// Decimal odds outside the representable range, in micro-units.
    ///
    /// Decimal odds include the stake, so 1.0 is the floor: it means the bet
    /// returns exactly what was put in. The ceiling of 1_000_000.0 is the
    /// reciprocal of the smallest non-zero probability, and holding to it is
    /// what lets `Odds -> Prob` stay total.
    #[error("decimal odds {0} micro-units is outside 1.0..=1_000_000.0")]
    OddsOutOfRange(u64),

    /// American odds strictly between -100 and +100.
    ///
    /// The notation has a hole there — it cannot express a payout between
    /// even money and even money — so a value in that range is a parsing bug,
    /// not an unusual price.
    #[error("american odds {0} is in the undefined range (-100, +100)")]
    AmericanOutOfRange(i32),

    /// A fractional quote with a zero numerator or denominator.
    #[error("fractional odds {0}/{1} is not a valid quote")]
    InvalidFractional(u32, u32),

    /// A non-finite or out-of-range floating point quote at the feed boundary.
    #[error("quote {0} is not a usable decimal price")]
    UnusableQuote(f64),

    /// A leg count outside `2..=MAX_LEGS`.
    ///
    /// A one-legged "arbitrage" is a directional bet; more legs than a market
    /// has outcomes means the caller assembled the market wrong.
    #[error("arbitrage needs 2..=4 legs, got {0}")]
    LegCountOutOfRange(usize),

    /// A stake increment of zero, which would admit infinitely divisible stakes.
    #[error("leg {0} has a stake increment of zero")]
    ZeroStakeIncrement(usize),
}

/// Shorthand for results carrying an [`ArbError`].
pub type Result<T> = core::result::Result<T, ArbError>;
