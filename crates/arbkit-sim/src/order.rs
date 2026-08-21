//! Order execution data structures and per-leg fill statuses.

use arbkit_core::{Cents, Fee, OutcomeId, Prob, VenueId, PPM};

/// One basis point denominator.
const BPS: u64 = 10_000;

/// Kalshi's exact published per-order settlement fee, in whole cents.
///
/// `crate::fee::kalshi_stake_fee_bps` (in `arbkit-core`) is the continuous
/// form used at *detection* time — it is a floor on the real charge and
/// stays authoritative there. This is the exact charge Kalshi actually
/// assesses per filled order, applied only at fill accounting so the
/// simulator's realized PnL matches what the venue really bills.
///
/// `stake_cents` is the cash paid for the order (the same quantity
/// [`LegFillResult::compute_fill`] calls `filled_stake`). Kalshi contracts pay
/// $1 each, so the number of contracts a cash stake buys at price `P` is the
/// notional it returns divided by $1: `C = (stake_cents / P) / 100`, i.e. the
/// order's gross payout in whole dollars. The published per-order fee is then
///
/// ```text
/// fee_cents = ceil( C * p_ppm * (PPM - p_ppm) * 7 / 1_000_000_000_000 )
/// ```
///
/// which peaks at 50 cents: 100 contracts bought at 50c cost $50 cash and
/// settle to exactly 175 cents of fee, matching Kalshi's published
/// $1.75-per-100-contracts ceiling. Contract count floors (matching the
/// payout-floors rounding rule), and the final division always rounds up
/// (`div_ceil`) so fill-time accounting never charges less than the venue
/// actually will.
///
/// `u128` throughout avoids overflow: `C` and `p_ppm * (PPM - p_ppm)` can each
/// be large, and their product times 7 would overflow `u64` well within
/// plausible stakes.
#[inline]
pub fn kalshi_exact_settlement_fee_cents(stake_cents: Cents, price: Prob) -> Cents {
    if stake_cents <= 0 {
        return 0;
    }
    let p_ppm = u128::from(price.ppm());
    let complement = u128::from(PPM) - p_ppm;
    // Contracts bought = gross payout in whole dollars = (stake / P) / 100,
    // combined into one division to avoid compounding rounding error.
    let contracts = (stake_cents as u128 * u128::from(PPM)) / (100 * p_ppm);
    let fee = (contracts * p_ppm * complement * 7).div_ceil(1_000_000_000_000);
    fee as Cents
}

/// A simulated order leg submitted to a venue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimulatedLegOrder {
    /// Target venue.
    pub venue: VenueId,
    /// Backed outcome.
    pub outcome: OutcomeId,
    /// Expected price at detection time.
    pub target_price: Prob,
    /// Fee schedule charged by venue.
    pub fee: Fee,
    /// Sized stake in cents.
    pub requested_stake: Cents,
    /// Minimum contract increment in cents.
    pub increment: Cents,
}

/// Reason why an order leg could not be filled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnfilledReason {
    /// The resting price moved away or worsened beyond the limit.
    PriceMoved {
        /// Price expected at detection time.
        expected: Prob,
        /// Price currently resting upon arrival (`None` if book empty).
        current: Option<Prob>,
    },
    /// Resting depth was insufficient or eaten by competitors before arrival.
    DepthExhausted {
        /// Effective depth remaining upon arrival.
        available: Cents,
        /// Stake requested by the order.
        requested: Cents,
    },
    /// The venue order book was marked stale upon arrival.
    BookStale,
    /// Sized stake is smaller than the minimum tradeable increment.
    IncrementConstraint,
}

/// Reason why an order leg was only partially filled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartialFillReason {
    /// Resting depth ran out before fulfilling the full stake.
    DepthDepleted,
    /// Stake was rounded down to comply with contract increment rules.
    IncrementRounding,
}

/// The fill status of an individual order leg.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegFillStatus {
    /// Fully filled for 100% of requested stake.
    Filled,
    /// Partially filled.
    PartiallyFilled {
        /// Sized stake filled in cents.
        filled_stake: Cents,
        /// Sized stake unfilled in cents.
        unfilled_stake: Cents,
        /// Cause of partial fill.
        reason: PartialFillReason,
    },
    /// Completely unfilled (0% fill).
    Unfilled(UnfilledReason),
}

/// Result of attempting to execute an individual order leg.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegFillResult {
    /// Venue ID.
    pub venue: VenueId,
    /// Outcome ID.
    pub outcome: OutcomeId,
    /// Execution status.
    pub status: LegFillStatus,
    /// The price at which the order executed.
    pub fill_price: Prob,
    /// Sized stake that was requested.
    pub requested_stake: Cents,
    /// Actual stake executed in cents.
    pub filled_stake: Cents,
    /// Gross payout if this outcome wins, before fee deductions.
    pub gross_payout: Cents,
    /// Venue fees deducted on this leg.
    pub fee_paid: Cents,
    /// Net payout if this outcome wins (`gross_payout - fee_paid`).
    pub net_payout: Cents,
    /// Simulated timestamp when the order arrived at the venue.
    pub arrival_timestamp_ns: u64,
}

impl LegFillResult {
    /// Create a fill result for an unfilled leg.
    pub const fn unfilled(
        venue: VenueId,
        outcome: OutcomeId,
        requested_stake: Cents,
        target_price: Prob,
        reason: UnfilledReason,
        arrival_timestamp_ns: u64,
    ) -> Self {
        Self {
            venue,
            outcome,
            status: LegFillStatus::Unfilled(reason),
            fill_price: target_price,
            requested_stake,
            filled_stake: 0,
            gross_payout: 0,
            fee_paid: 0,
            net_payout: 0,
            arrival_timestamp_ns,
        }
    }

    /// Calculate gross payout, fee deduction, and net payout for a filled stake.
    ///
    /// Always follows pessimistic integer rounding rules:
    /// - Gross payout floors: `floor(stake * PPM / fill_price)`.
    /// - Stake fees ceil: `ceil(stake * bps / 10000)`.
    /// - Commission ceils on net winnings: `ceil(winnings * bps / 10000)`.
    #[allow(clippy::too_many_arguments)]
    pub fn compute_fill(
        venue: VenueId,
        outcome: OutcomeId,
        requested_stake: Cents,
        filled_stake: Cents,
        fill_price: Prob,
        fee: Fee,
        status: LegFillStatus,
        arrival_timestamp_ns: u64,
    ) -> Self {
        if filled_stake <= 0 {
            return Self::unfilled(
                venue,
                outcome,
                requested_stake,
                fill_price,
                UnfilledReason::DepthExhausted {
                    available: 0,
                    requested: requested_stake,
                },
                arrival_timestamp_ns,
            );
        }

        let gross_payout =
            ((filled_stake as i128 * PPM as i128) / (fill_price.ppm() as i128)) as Cents;
        let effective_price = fee.effective(fill_price);
        let net_payout =
            ((filled_stake as i128 * PPM as i128) / (effective_price.ppm() as i128)) as Cents;
        let fee_paid = gross_payout.saturating_sub(net_payout);

        Self {
            venue,
            outcome,
            status,
            fill_price,
            requested_stake,
            filled_stake,
            gross_payout,
            fee_paid,
            net_payout,
            arrival_timestamp_ns,
        }
    }

    /// Like [`Self::compute_fill`], but for a Kalshi leg specifically: the fee
    /// component uses [`kalshi_exact_settlement_fee_cents`], the exact
    /// published per-order charge, instead of the continuous
    /// [`Fee::StakeFeeBps`] approximation used generically by
    /// `compute_fill`. `Fee::StakeFeeBps` (built from
    /// `arbkit_core::fee::kalshi_stake_fee_bps`) remains what *detection*
    /// compares against — it is deliberately a floor on the real charge — so
    /// this is only ever called at fill-time accounting, and only for orders
    /// actually routed to Kalshi.
    ///
    /// Callers wiring up Kalshi fills should call this instead of
    /// `compute_fill`; it is not dispatched automatically from a `Fee` value
    /// because `Fee::StakeFeeBps` is also used, with an unrelated bps figure,
    /// by non-Kalshi venues (see module docs on `Fee::StakeFeeBps`), for
    /// which this exact formula does not apply.
    pub fn compute_fill_kalshi_exact(
        venue: VenueId,
        outcome: OutcomeId,
        requested_stake: Cents,
        filled_stake: Cents,
        fill_price: Prob,
        status: LegFillStatus,
        arrival_timestamp_ns: u64,
    ) -> Self {
        if filled_stake <= 0 {
            return Self::unfilled(
                venue,
                outcome,
                requested_stake,
                fill_price,
                UnfilledReason::DepthExhausted {
                    available: 0,
                    requested: requested_stake,
                },
                arrival_timestamp_ns,
            );
        }

        let gross_payout =
            ((filled_stake as i128 * PPM as i128) / (fill_price.ppm() as i128)) as Cents;
        let fee_paid = kalshi_exact_settlement_fee_cents(filled_stake, fill_price);
        let net_payout = gross_payout.saturating_sub(fee_paid);

        Self {
            venue,
            outcome,
            status,
            fill_price,
            requested_stake,
            filled_stake,
            gross_payout,
            fee_paid,
            net_payout,
            arrival_timestamp_ns,
        }
    }

    /// Whether the leg was completely filled.
    #[inline]
    pub const fn is_fully_filled(&self) -> bool {
        matches!(self.status, LegFillStatus::Filled)
    }

    /// Whether the leg was partially filled.
    #[inline]
    pub const fn is_partially_filled(&self) -> bool {
        matches!(self.status, LegFillStatus::PartiallyFilled { .. })
    }

    /// Whether the leg was completely unfilled.
    #[inline]
    pub const fn is_unfilled(&self) -> bool {
        matches!(self.status, LegFillStatus::Unfilled(_))
    }

    /// Fill ratio in basis points (`filled_stake * 10000 / requested_stake`).
    #[inline]
    pub fn fill_ratio_bps(&self) -> u32 {
        if self.requested_stake <= 0 {
            return 0;
        }
        ((self.filled_stake as u128 * BPS as u128) / (self.requested_stake as u128)) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arbkit_core::kalshi_stake_fee_bps;
    use proptest::prelude::*;

    #[test]
    fn compute_fill_gross_and_net_payout() {
        let price = Prob::from_cents(50).unwrap();
        // $100 stake at 50c price -> $200 gross payout
        let res = LegFillResult::compute_fill(
            1,
            0,
            10_000,
            10_000,
            price,
            Fee::CommissionBps(500), // 5% on winnings ($100 winnings -> $5 fee)
            LegFillStatus::Filled,
            1_000_000,
        );

        assert_eq!(res.gross_payout, 20_000);
        assert_eq!(res.fee_paid, 501);
        assert_eq!(res.net_payout, 19_499);
        assert_eq!(res.fill_ratio_bps(), 10_000);
        assert!(res.is_fully_filled());
    }

    #[test]
    fn kalshi_exact_fee_hits_the_published_ceiling() {
        // 100 contracts at 50c cost $50 cash and settle to exactly $1.75 of
        // fee: Kalshi's published $1.75-per-100-contracts ceiling.
        let price = Prob::from_cents(50).unwrap();
        assert_eq!(kalshi_exact_settlement_fee_cents(5_000, price), 175);
    }

    #[test]
    fn kalshi_exact_fee_table_matches_cheap_and_dear_contracts() {
        // Same three price points as `fee::tests::kalshi_fee_is_worse_on_cheap_contracts`
        // (crates/arbkit-core/src/fee.rs:159-166), each sized so the cash
        // stake buys exactly 100 contracts, so the absolute fee is directly
        // comparable across prices. The raw fee is symmetric in `P` and
        // `1 - P` (it depends on `P * (1 - P)`), so cheap and dear contracts
        // land on the same cents figure, unlike the bps view where cheap
        // contracts look far more expensive.
        let cheap = Prob::from_cents(5).unwrap(); // 100 contracts cost $5
        let even = Prob::from_cents(50).unwrap(); // 100 contracts cost $50
        let dear = Prob::from_cents(95).unwrap(); // 100 contracts cost $95

        assert_eq!(kalshi_exact_settlement_fee_cents(500, cheap), 34);
        assert_eq!(kalshi_exact_settlement_fee_cents(5_000, even), 175);
        assert_eq!(kalshi_exact_settlement_fee_cents(9_500, dear), 34);
    }

    #[test]
    fn kalshi_exact_fee_is_zero_for_a_nonpositive_stake() {
        let price = Prob::from_cents(50).unwrap();
        assert_eq!(kalshi_exact_settlement_fee_cents(0, price), 0);
        assert_eq!(kalshi_exact_settlement_fee_cents(-100, price), 0);
    }

    #[test]
    fn compute_fill_kalshi_exact_matches_the_standalone_fee_function() {
        let price = Prob::from_cents(50).unwrap();
        let res = LegFillResult::compute_fill_kalshi_exact(
            1,
            0,
            5_000,
            5_000,
            price,
            LegFillStatus::Filled,
            1_000_000,
        );

        assert_eq!(res.gross_payout, 10_000);
        assert_eq!(res.fee_paid, 175);
        assert_eq!(res.net_payout, 9_825);
    }

    proptest! {
        /// The exact per-order Kalshi fee must never be cheaper than the
        /// continuous form (`kalshi_stake_fee_bps`) detection assumed for the
        /// same cash stake — preserving pessimism end-to-end from detection
        /// through fill accounting. `kalshi_stake_fee_bps` is documented as a
        /// floor on the real charge; this checks that the exact fee-time
        /// computation never undercuts that floor.
        ///
        /// `stake_cents` is generated as an exact `contracts * price_cents`,
        /// matching how a real fill's stake always arises (an integer number
        /// of whole contracts bought at a quoted price): a `stake_cents` with
        /// no integer-contracts interpretation isn't a stake accounting ever
        /// sees, and asking `kalshi_exact_settlement_fee_cents` to floor a
        /// stake that was never whole contracts to begin with can — by
        /// construction — chip a cent off the exact fee relative to the
        /// continuous estimate (this is exactly what the regression seed in
        /// `crates/arbkit-sim/proptest-regressions/order.txt` found).
        #[test]
        fn exact_fee_never_cheaper_than_continuous_estimate(
            contracts in 1i64..10_000_000i64,
            price_cents in 1u32..=99u32,
        ) {
            let price = Prob::from_cents(price_cents).unwrap();
            let stake_cents = contracts * i64::from(price_cents);
            let exact = kalshi_exact_settlement_fee_cents(stake_cents, price);
            let continuous_estimate =
                (stake_cents as i128 * i128::from(kalshi_stake_fee_bps(price)) / 10_000) as Cents;
            prop_assert!(exact >= continuous_estimate);
        }
    }
}
