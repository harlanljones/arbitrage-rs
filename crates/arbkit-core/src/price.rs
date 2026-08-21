//! Prices, in the one representation the whole system agrees on.
//!
//! Venues quote the same number four different ways. A US sportsbook says
//! `-110`, a European one says `1.91`, a UK one says `10/11`, and Kalshi says
//! `52` cents. All four are the same claim about the world, and comparing them
//! is the first thing an arbitrage engine has to do.
//!
//! Everything is normalized to [`Prob`], an implied probability in parts per
//! million held in a `u32`, and its reciprocal [`Odds`], decimal odds in
//! micro-units held in a `u64`. The two are exact reciprocals through the
//! constant `1e12`:
//!
//! ```text
//! prob_ppm x odds_micro ~= 1_000_000_000_000
//! ```
//!
//! # Why integers
//!
//! Arbitrage is decided by whether a sum of implied probabilities lands just
//! under 1.0. Real edges are tens of basis points wide, and the arithmetic
//! that produces them is a chain of reciprocals. Doing that chain in `f64`
//! means the answer depends on the order of the additions, and the errors
//! accumulate in exactly the region where the decision is made — so a
//! rounding artifact can manufacture an edge that was never on the screen.
//! Fixed-point integers make the comparison exact and reproducible, which also
//! means a replayed tape produces bit-identical signals.
//!
//! Floating point is confined to the feed boundary, where JSON hands us an
//! `f64` and we have no choice; those constructors are named `_f64` so they
//! are easy to grep for and hard to reach for by accident.

use crate::error::{ArbError, Result};

/// One whole unit of probability, in parts per million.
pub const PPM: u32 = 1_000_000;

/// One whole unit of decimal odds, in micro-units.
pub const ODDS_ONE: u64 = 1_000_000;

/// The product a [`Prob`] and its [`Odds`] must multiply to.
const RECIPROCAL: u64 = PPM as u64 * ODDS_ONE;

/// Divide, rounding to nearest rather than truncating.
///
/// Truncation biases every conversion in the same direction, and a systematic
/// downward bias on implied probabilities is indistinguishable from free
/// money. Rounding to nearest keeps the error zero-mean and under half a ppm.
#[inline]
const fn div_round(numerator: u64, denominator: u64) -> u64 {
    (numerator + denominator / 2) / denominator
}

/// An implied probability in parts per million, in `1..=1_000_000`.
///
/// This is the canonical price type. Ordering is probability ordering: a
/// *larger* `Prob` is a *shorter* price and a worse payout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Prob(u32);

impl Prob {
    /// The shortest representable price: a certainty, paying nothing.
    pub const CERTAIN: Prob = Prob(PPM);

    /// Build from parts per million.
    #[inline]
    pub const fn from_ppm(ppm: u32) -> Result<Prob> {
        if ppm == 0 || ppm > PPM {
            return Err(ArbError::ProbOutOfRange(ppm));
        }
        Ok(Prob(ppm))
    }

    /// The underlying parts per million.
    #[inline]
    pub const fn ppm(self) -> u32 {
        self.0
    }

    /// Build from American ("moneyline") odds, as `-110` or `+150`.
    ///
    /// Values strictly between -100 and +100 are rejected: the notation has no
    /// meaning there.
    pub const fn from_american(american: i32) -> Result<Prob> {
        let magnitude = american.unsigned_abs() as u64;
        if magnitude < 100 {
            return Err(ArbError::AmericanOutOfRange(american));
        }
        // Favourite (-110): risk 110 to win 100, so p = 110 / 210.
        // Underdog (+150): risk 100 to win 150, so p = 100 / 250.
        let ppm = if american < 0 {
            div_round(magnitude * PPM as u64, magnitude + 100)
        } else {
            div_round(100 * PPM as u64, magnitude + 100)
        };
        Prob::from_ppm(ppm as u32)
    }

    /// Render as American odds, or `None` for a price with no representation.
    ///
    /// [`Prob::CERTAIN`] returns `None` — a bet that pays exactly the stake
    /// back cannot be written in a notation built around the payout above the
    /// stake. Even money renders as `+100` by convention.
    pub const fn to_american(self) -> Option<i32> {
        let p = self.0 as u64;
        let complement = PPM as u64 - p;
        if complement == 0 {
            return None;
        }
        if p > PPM as u64 / 2 {
            Some(-(div_round(100 * p, complement) as i32))
        } else {
            Some(div_round(100 * complement, p) as i32)
        }
    }

    /// Build from fractional odds, as `10/11` for `-110`.
    pub const fn from_fractional(numerator: u32, denominator: u32) -> Result<Prob> {
        if numerator == 0 || denominator == 0 {
            return Err(ArbError::InvalidFractional(numerator, denominator));
        }
        // Fractional n/d wins n for every d risked, so p = d / (n + d).
        let total = numerator as u64 + denominator as u64;
        let ppm = div_round(denominator as u64 * PPM as u64, total);
        Prob::from_ppm(ppm as u32)
    }

    /// Build from a whole-cent price in `1..=99`, the way Kalshi quotes.
    ///
    /// A contract bought at 52 cents settles at 100 cents if it resolves yes,
    /// so the quote *is* the implied probability.
    pub const fn from_cents(cents: u32) -> Result<Prob> {
        if cents == 0 || cents >= 100 {
            return Err(ArbError::ProbOutOfRange(cents * 10_000));
        }
        Prob::from_ppm(cents * 10_000)
    }

    /// The reciprocal view: decimal odds.
    #[inline]
    pub const fn to_odds(self) -> Odds {
        Odds(div_round(RECIPROCAL, self.0 as u64))
    }

    /// Build from a decimal quote arriving as a float.
    ///
    /// Feed-boundary only. Once a quote is a `Prob`, keep it one.
    pub fn from_decimal_f64(decimal: f64) -> Result<Prob> {
        Odds::from_decimal_f64(decimal).map(Odds::to_prob)
    }

    /// This price as a float, for display and reporting.
    #[inline]
    pub fn as_f64(self) -> f64 {
        f64::from(self.0) / f64::from(PPM)
    }
}

/// Decimal odds in micro-units, at or above 1.0.
///
/// Decimal odds are stake-inclusive: a winning stake of `s` at odds `d`
/// returns `s * d`, of which `s` was already yours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Odds(u64);

impl Odds {
    /// The longest representable price, the reciprocal of one part per million.
    pub const LONGEST: Odds = Odds(RECIPROCAL);

    /// Build from micro-units, where `1_910_000` is decimal `1.91`.
    #[inline]
    pub const fn from_micro(micro: u64) -> Result<Odds> {
        if micro < ODDS_ONE || micro > RECIPROCAL {
            return Err(ArbError::OddsOutOfRange(micro));
        }
        Ok(Odds(micro))
    }

    /// The underlying micro-units.
    #[inline]
    pub const fn micro(self) -> u64 {
        self.0
    }

    /// The reciprocal view: implied probability.
    ///
    /// Total by construction: [`Odds::from_micro`] bounds the odds to
    /// `1.0..=1_000_000.0`, which is exactly the range whose reciprocal lands
    /// inside `1..=1_000_000` ppm. That is why the ceiling exists.
    #[inline]
    pub const fn to_prob(self) -> Prob {
        Prob(div_round(RECIPROCAL, self.0) as u32)
    }

    /// Build from a decimal quote arriving as a float. Feed-boundary only.
    pub fn from_decimal_f64(decimal: f64) -> Result<Odds> {
        if !decimal.is_finite() || decimal < 1.0 {
            return Err(ArbError::UnusableQuote(decimal));
        }
        let micro = (decimal * ODDS_ONE as f64).round();
        if micro > RECIPROCAL as f64 {
            return Err(ArbError::UnusableQuote(decimal));
        }
        Odds::from_micro(micro as u64)
    }

    /// These odds as a float, for display and reporting.
    #[inline]
    pub fn as_f64(self) -> f64 {
        self.0 as f64 / ODDS_ONE as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn american_favourite_matches_published_implied_probability() {
        // -110 is the standard US sportsbook price: risk 110 to win 100.
        let p = Prob::from_american(-110).unwrap();
        assert_eq!(p.ppm(), 523_810);
    }

    #[test]
    fn american_underdog_matches_published_implied_probability() {
        assert_eq!(Prob::from_american(150).unwrap().ppm(), 400_000);
        assert_eq!(Prob::from_american(100).unwrap().ppm(), 500_000);
        assert_eq!(Prob::from_american(-200).unwrap().ppm(), 666_667);
    }

    #[test]
    fn american_notation_has_a_hole_around_even_money() {
        assert!(Prob::from_american(0).is_err());
        assert!(Prob::from_american(99).is_err());
        assert!(Prob::from_american(-99).is_err());
    }

    #[test]
    fn fractional_and_american_agree_on_the_same_price() {
        // 10/11 is the UK way of writing -110.
        let fractional = Prob::from_fractional(10, 11).unwrap();
        let american = Prob::from_american(-110).unwrap();
        assert_eq!(fractional, american);
    }

    #[test]
    fn cents_are_already_probabilities() {
        assert_eq!(Prob::from_cents(52).unwrap().ppm(), 520_000);
        // A contract cannot trade at 0 or 100: there would be nothing to win.
        assert!(Prob::from_cents(0).is_err());
        assert!(Prob::from_cents(100).is_err());
    }

    #[test]
    fn decimal_quotes_convert_at_the_boundary() {
        let p = Prob::from_decimal_f64(1.91).unwrap();
        assert_eq!(p.to_american(), Some(-110));
    }

    #[test]
    fn a_certainty_has_no_american_representation() {
        assert_eq!(Prob::CERTAIN.to_american(), None);
        // ...but it is still a valid price, and still converts to odds of 1.0.
        assert_eq!(Prob::CERTAIN.to_odds().micro(), ODDS_ONE);
    }

    #[test]
    fn larger_prob_is_a_shorter_price() {
        let favourite = Prob::from_american(-200).unwrap();
        let underdog = Prob::from_american(200).unwrap();
        assert!(favourite > underdog);
        assert!(favourite.to_odds() < underdog.to_odds());
    }

    #[test]
    fn odds_are_bounded_so_the_reciprocal_stays_a_valid_price() {
        assert!(Odds::from_micro(999_999).is_err());
        assert_eq!(Odds::LONGEST.to_prob().ppm(), 1);
        assert!(Odds::from_micro(Odds::LONGEST.micro() + 1).is_err());
    }
}
