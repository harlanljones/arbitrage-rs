//! Depth discounting for fill-time sizing.
//!
//! [`crate::arb::detect`] sizes a signal against resting depth as reported by
//! the book at detection time. By the time an order actually reaches a venue,
//! some of that depth is gone — eaten by faster competitors, or simply
//! decayed while the signal was in flight. `arbkit-sim`'s
//! `LatencyProfile::effective_depth` already models this decay for the
//! simulator; [`DepthDiscount`] is the same integer formula, moved into
//! `arbkit-core` so that fill-time checks on the hot path can apply it
//! without depending on the simulator crate (which is async, allocates, and
//! is not on the hot path at all).
//!
//! Keeping the two formulas bit-for-bit identical matters: if detection-side
//! sizing and fill-time sizing round differently, the two halves of the
//! system disagree about how much depth was ever really there, and one of
//! them is lying. Both floor. Neither ever reports more usable depth than the
//! raw book actually held.

use crate::{Cents, Level, MAX_LEVELS};

/// One basis point (0.01%), i.e. the denominator for [`DepthDiscount::survival_bps`].
const BPS: u64 = 10_000;

/// Share of resting depth expected to survive transit to the venue, in basis
/// points of the raw resting size.
///
/// `10_000` means the depth is untouched by the time the order arrives;
/// `0` means none of it survives. Values above `10_000` are clamped down to
/// `10_000` rather than treated as an error — a discount can never manufacture
/// depth that was not quoted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepthDiscount {
    /// Survival rate, in basis points of the raw resting size.
    pub survival_bps: u32,
}

impl DepthDiscount {
    /// No discount: all resting depth is assumed to survive.
    pub const NONE: DepthDiscount = DepthDiscount {
        survival_bps: 10_000,
    };

    /// Total discount: none of the resting depth is assumed to survive.
    pub const TOTAL: DepthDiscount = DepthDiscount { survival_bps: 0 };

    /// Pessimistic (floored) usable depth after applying the discount.
    ///
    /// Zero (or negative — a malformed book) `raw_depth` always yields zero;
    /// there is nothing to discount. Otherwise the result is floored, so this
    /// never reports more usable depth than the raw book actually held. This
    /// mirrors `LatencyProfile::effective_depth` in `arbkit-sim` exactly:
    /// same clamp, same `u128` intermediate, same floor.
    #[inline]
    pub fn discounted(&self, raw_depth: Cents) -> Cents {
        if raw_depth <= 0 {
            return 0;
        }
        let bps = u64::from(self.survival_bps).min(BPS);
        ((raw_depth as u128 * bps as u128) / BPS as u128) as Cents
    }

    /// Discount every level of a book slice, in place semantics but without
    /// allocation: the result is a fixed-size array (matching
    /// [`crate::book::OutcomeBook`]'s own storage) paired with the number of
    /// levels actually filled in.
    ///
    /// Levels beyond [`MAX_LEVELS`] are dropped, same as
    /// [`crate::book::OutcomeBook::apply_snapshot`] — depth past the top few
    /// levels is not sized against on the hot path, so there is nothing to
    /// gain by carrying more of it through here.
    pub fn discounted_levels(&self, levels: &[Level]) -> ([Level; MAX_LEVELS], usize) {
        let mut out = [Level {
            price: crate::price::Prob::CERTAIN,
            size: 0,
        }; MAX_LEVELS];
        let take = levels.len().min(MAX_LEVELS);
        for (dst, src) in out[..take].iter_mut().zip(&levels[..take]) {
            dst.price = src.price;
            dst.size = self.discounted(src.size);
        }
        (out, take)
    }
}

impl Default for DepthDiscount {
    /// Defaults to no discount: callers that never configured a discount
    /// should see the raw book, not a silently zeroed one.
    fn default() -> Self {
        Self::NONE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::price::Prob;

    fn level(cents: u32, size: Cents) -> Level {
        Level {
            price: Prob::from_cents(cents).unwrap(),
            size,
        }
    }

    #[test]
    fn zero_survival_zeroes_depth() {
        let discount = DepthDiscount { survival_bps: 0 };
        assert_eq!(discount.discounted(100_000), 0);
    }

    #[test]
    fn partial_survival_floors() {
        // 25% front-run in the LatencyProfile framing is 7500 bps survival.
        let discount = DepthDiscount {
            survival_bps: 7_500,
        };
        assert_eq!(discount.discounted(100_000), 75_000);

        // A case that does not divide evenly must floor, not round.
        let discount = DepthDiscount {
            survival_bps: 3_333,
        };
        // 10_000 * 3_333 / 10_000 = 3_333.0 exactly, pick a depth that does not.
        assert_eq!(discount.discounted(7), 2); // 7*3333/10000 = 2.3331 -> 2
    }

    #[test]
    fn full_survival_is_untouched() {
        let discount = DepthDiscount::NONE;
        assert_eq!(discount.discounted(123_456), 123_456);
    }

    #[test]
    fn survival_above_10_000_bps_is_clamped_not_amplified() {
        let discount = DepthDiscount {
            survival_bps: 50_000,
        };
        assert_eq!(discount.discounted(1_000), 1_000);
    }

    #[test]
    fn zero_and_negative_depth_are_always_zero() {
        let discount = DepthDiscount::NONE;
        assert_eq!(discount.discounted(0), 0);
        assert_eq!(discount.discounted(-500), 0);

        let total = DepthDiscount::TOTAL;
        assert_eq!(total.discounted(0), 0);
        assert_eq!(total.discounted(-500), 0);
    }

    #[test]
    fn discounted_levels_preserves_price_and_discounts_size() {
        let discount = DepthDiscount {
            survival_bps: 2_500,
        };
        let levels = [level(52, 1_000), level(55, 3), level(60, 0)];
        let (out, len) = discount.discounted_levels(&levels);
        assert_eq!(len, 3);
        assert_eq!(out[0].price, levels[0].price);
        assert_eq!(out[0].size, 250);
        assert_eq!(out[1].price, levels[1].price);
        assert_eq!(out[1].size, 0); // 3 * 2500 / 10_000 = 0.75 -> floors to 0
        assert_eq!(out[2].size, 0);
    }

    #[test]
    fn discounted_levels_truncates_at_capacity_like_outcomebook() {
        let discount = DepthDiscount::NONE;
        let deep: Vec<Level> = (1..=20).map(|i| level(i + 10, 100)).collect();
        let (_out, len) = discount.discounted_levels(&deep);
        assert_eq!(len, MAX_LEVELS);
    }

    /// Reference implementation of `LatencyProfile::effective_depth`'s
    /// formula (`arbkit-sim/src/latency.rs`), copied inline rather than
    /// imported so `arbkit-core` never depends on `arbkit-sim`. This is the
    /// contract [`DepthDiscount::discounted`] must match bit-for-bit: the
    /// simulator's "percent of depth eaten by front-runners" and this
    /// module's "percent of depth expected to survive" are the same knob
    /// read from opposite ends (`front_run_bps` vs `survival_bps = 10_000 -
    /// front_run_bps`), so equivalence here is what keeps detection-side
    /// sizing and fill-time sizing from silently disagreeing.
    fn reference_effective_depth(raw_depth: Cents, front_run_bps: u32) -> Cents {
        const REF_BPS: u64 = 10_000;
        if raw_depth <= 0 {
            return 0;
        }
        let bps = u64::from(front_run_bps).min(REF_BPS);
        let remaining_bps = REF_BPS - bps;
        ((raw_depth as u128 * remaining_bps as u128) / REF_BPS as u128) as Cents
    }

    #[test]
    fn matches_latency_profile_effective_depth_across_a_grid() {
        let depths: [Cents; 7] = [0, -10, 1, 7, 1_000, 100_000, i64::MAX / 2];
        let front_run_bps_values: [u32; 9] =
            [0, 1, 100, 2_500, 3_333, 5_000, 9_999, 10_000, 20_000];

        for &raw_depth in &depths {
            for &front_run_bps in &front_run_bps_values {
                let survival_bps = 10_000u32.saturating_sub(front_run_bps.min(10_000));
                let discount = DepthDiscount { survival_bps };
                assert_eq!(
                    discount.discounted(raw_depth),
                    reference_effective_depth(raw_depth, front_run_bps),
                    "mismatch at raw_depth={raw_depth}, front_run_bps={front_run_bps}"
                );
            }
        }
    }
}
