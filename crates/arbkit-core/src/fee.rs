//! What the venue takes, applied before the comparison rather than after.
//!
//! A cross-venue price sum of 0.997 looks like 30 basis points of free money
//! and is, on most real venues, a loss. Betfair takes a commission on net
//! winnings; Kalshi charges a per-contract fee that peaks at 3.5% of stake;
//! a sportsbook's rake is already inside the quoted price but its withdrawal
//! costs are not. Detecting on raw quotes and subtracting costs afterwards
//! produces a signal stream dominated by trades that were never profitable.
//!
//! So fees live here, as a transformation from a quoted [`Prob`] to the
//! *effective* [`Prob`] the arbitrage condition is actually evaluated on. By
//! the time [`crate::arb::detect`] sums anything, the costs are already in the
//! numbers.

use crate::price::{Prob, ODDS_ONE, PPM};

/// One basis point's worth of denominator.
const BPS: u64 = 10_000;

/// How a venue charges for a filled bet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fee {
    /// The quoted price is the whole cost.
    #[default]
    None,

    /// A percentage of *net winnings*, in basis points — the exchange model.
    ///
    /// Betfair-style. Decimal odds `d` become `1 + (d - 1) * (1 - c)`: the
    /// stake comes back untaxed and only the profit is charged. This bites
    /// hardest on long prices, where nearly all of the return is profit.
    CommissionBps(u32),

    /// A percentage of *stake*, in basis points.
    ///
    /// Decimal odds `d` become `d / (1 + f)`. Use this for any cost incurred
    /// on entry regardless of outcome, which is how a per-contract fee
    /// behaves once amortized over the contracts bought.
    StakeFeeBps(u32),

    /// Venue *pays* this many bps of stake for resting liquidity, the maker
    /// rebate model.
    ///
    /// The mirror image of [`Fee::StakeFeeBps`]: the effective price shrinks
    /// rather than grows. Floored rather than rounded, so the rebate never
    /// overstates the benefit, and saturating at 1 ppm rather than 0 — a
    /// price of exactly zero is not a representable [`Prob`] and would imply
    /// an infinite decimal price, which no real rebate can produce.
    MakerRebateBps(u32),
}

impl Fee {
    /// The effective price after this venue's cut.
    ///
    /// Always at least as short as the quoted price — a fee can only ever make
    /// a bet worse. Saturates at [`Prob::CERTAIN`] when the fee swallows the
    /// entire edge, which [`crate::arb::detect`] then rejects on the sum.
    #[inline]
    pub fn effective(self, quoted: Prob) -> Prob {
        match self {
            Fee::None => quoted,
            Fee::CommissionBps(0) | Fee::StakeFeeBps(0) => quoted,

            Fee::CommissionBps(bps) => {
                let bps = u64::from(bps).min(BPS);
                let decimal = quoted.to_odds().micro();
                // Only the part above the returned stake is charged.
                let profit_part = decimal - ODDS_ONE;
                let net = ODDS_ONE + profit_part * (BPS - bps) / BPS;
                // `net >= ODDS_ONE` holds, so this stays a valid price.
                Prob::from_ppm(
                    (u64::from(PPM) * ODDS_ONE)
                        .div_ceil(net)
                        .min(u64::from(PPM)) as u32,
                )
                .unwrap_or(Prob::CERTAIN)
            }

            Fee::StakeFeeBps(bps) => {
                let bps = u64::from(bps);
                let inflated = u64::from(quoted.ppm()) * (BPS + bps) / BPS;
                Prob::from_ppm(inflated.min(u64::from(PPM)) as u32).unwrap_or(Prob::CERTAIN)
            }

            Fee::MakerRebateBps(0) => quoted,

            Fee::MakerRebateBps(bps) => {
                let bps = u64::from(bps).min(BPS);
                // Floor, matching the pessimistic-rounding rule: the rebate
                // never gets credited for more than it is actually worth.
                let shrunk = u64::from(quoted.ppm()) * (BPS - bps) / BPS;
                // Saturate at 1 ppm rather than 0 — `Prob` cannot represent a
                // zero price, and an unbounded rebate cannot make a bet free.
                Prob::from_ppm(shrunk.max(1) as u32).unwrap_or(Prob::from_ppm(1).unwrap())
            }
        }
    }
}

/// Kalshi's per-contract fee, expressed as basis points of stake.
///
/// Kalshi charges `ceil(0.07 * C * P * (1 - P))` dollars on an order of `C`
/// contracts at price `P`. The stake on that order is `C * P`, so the fee as a
/// fraction of stake collapses to a term that does not depend on size at all:
///
/// ```text
/// fee / stake = 0.07 * (1 - P)
/// ```
///
/// which is `700 * (1 - P)` basis points. It peaks at 350 bps for a 50-cent
/// contract, matching Kalshi's published $1.75-per-100-contracts ceiling, and
/// it is *larger* on cheap contracts — the opposite of the intuition that a
/// long shot is cheap to trade.
///
/// This is the continuous form and therefore a floor on the real charge, which
/// rounds up to the whole cent on each order.
#[inline]
pub fn kalshi_stake_fee_bps(price: Prob) -> u32 {
    let complement = u64::from(PPM - price.ppm());
    (700 * complement / u64::from(PPM)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn no_fee_leaves_the_price_alone() {
        let quoted = Prob::from_cents(52).unwrap();
        assert_eq!(Fee::None.effective(quoted), quoted);
        assert_eq!(Fee::CommissionBps(0).effective(quoted), quoted);
        assert_eq!(Fee::StakeFeeBps(0).effective(quoted), quoted);
    }

    #[test]
    fn commission_bites_hardest_on_long_prices() {
        // 5% on net winnings, the Betfair default.
        let fee = Fee::CommissionBps(500);
        let short = Prob::from_decimal_f64(1.10).unwrap();
        let long = Prob::from_decimal_f64(10.0).unwrap();

        // Measured as a share of the price itself, not in absolute ppm: the
        // two absolute costs are nearly identical, which is the trap. What
        // matters to an edge is the proportional haircut.
        let cost_bps = |quoted: Prob| {
            let effective = fee.effective(quoted).ppm();
            (u64::from(effective - quoted.ppm()) * BPS) / u64::from(quoted.ppm())
        };

        // Nearly all of a 10.0 return is profit, so nearly all of it is taxed;
        // at 1.10 the commission only touches the thin sliver above stake.
        assert_eq!(cost_bps(short), 45);
        assert_eq!(cost_bps(long), 471);
    }

    #[test]
    fn stake_fee_scales_the_price_directly() {
        // 3.5% of stake against an even-money price.
        let quoted = Prob::from_cents(50).unwrap();
        assert_eq!(Fee::StakeFeeBps(350).effective(quoted).ppm(), 517_500);
    }

    #[test]
    fn a_fee_can_never_improve_a_price() {
        for cents in 1..100 {
            let quoted = Prob::from_cents(cents).unwrap();
            for fee in [Fee::CommissionBps(500), Fee::StakeFeeBps(350)] {
                assert!(fee.effective(quoted) >= quoted, "{fee:?} at {cents}c");
            }
        }
    }

    #[test]
    fn kalshi_fee_peaks_at_the_published_ceiling() {
        // Kalshi caps at $1.75 per 100 contracts, reached at 50 cents. A
        // hundred contracts at 50c is $50 of stake, so 1.75/50 = 3.5%.
        assert_eq!(kalshi_stake_fee_bps(Prob::from_cents(50).unwrap()), 350);
    }

    #[test]
    fn kalshi_fee_is_worse_on_cheap_contracts() {
        // The intuition that a 5-cent long shot is cheap to trade is exactly
        // backwards: as a share of stake it is the most expensive thing there.
        let cheap = kalshi_stake_fee_bps(Prob::from_cents(5).unwrap());
        let even = kalshi_stake_fee_bps(Prob::from_cents(50).unwrap());
        let dear = kalshi_stake_fee_bps(Prob::from_cents(95).unwrap());
        assert_eq!((cheap, even, dear), (665, 350, 35));
    }

    #[test]
    fn maker_rebate_improves_stake_fee_and_pays_you_for_resting() {
        // 20 bps rebate against a 50c quote shrinks the effective price.
        let quoted = Prob::from_cents(50).unwrap();
        assert_eq!(Fee::MakerRebateBps(200).effective(quoted).ppm(), 490_000);
    }

    #[test]
    fn zero_rebate_leaves_the_price_alone() {
        let quoted = Prob::from_cents(52).unwrap();
        assert_eq!(Fee::MakerRebateBps(0).effective(quoted), quoted);
    }

    #[test]
    fn maker_rebate_saturates_at_one_ppm_rather_than_zero() {
        // A rebate large enough to zero out the price instead floors at the
        // smallest representable `Prob`, since `Prob` cannot be zero.
        let quoted = Prob::from_ppm(1).unwrap();
        assert_eq!(Fee::MakerRebateBps(10_000).effective(quoted).ppm(), 1);

        let cheap = Prob::from_cents(1).unwrap();
        assert_eq!(Fee::MakerRebateBps(10_000).effective(cheap).ppm(), 1);
    }

    proptest! {
        /// A maker rebate can only ever shrink the effective ppm relative to
        /// the quoted price, and never all the way to zero — mirroring
        /// `a_fee_can_never_improve_a_price` but in the other direction.
        #[test]
        fn maker_rebate_never_worsens_and_never_hits_zero(
            ppm in 1u32..=PPM,
            bps in 0u32..=20_000,
        ) {
            let quoted = Prob::from_ppm(ppm).unwrap();
            let effective = Fee::MakerRebateBps(bps).effective(quoted);
            prop_assert!(effective.ppm() <= quoted.ppm());
            prop_assert!(effective.ppm() >= 1);
        }
    }
}
