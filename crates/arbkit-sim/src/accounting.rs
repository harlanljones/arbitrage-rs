//! PnL, fee deduction, slippage, and fill accounting.
//!
//! All financial accounting uses whole cents ([`Cents`]) and fixed-point integer
//! arithmetic. Floating point is restricted to display/reporting helpers ending in `_f64`.

use arbkit_core::Cents;

use crate::order::LegFillResult;

/// One basis point denominator (100% = 10,000 bps).
const BPS: i128 = 10_000;

/// Profit and loss summary for a single arbitrage execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionPnl {
    /// Total stake requested in the original signal, in cents.
    pub requested_stake: Cents,
    /// Total actual stake committed across all filled legs, in cents.
    pub filled_stake: Cents,
    /// Sum of all venue fees deducted across all filled legs, in cents.
    pub total_fees: Cents,
    /// The worst-case payout across all potential winning outcomes, in cents.
    pub worst_case_payout: Cents,
    /// Worst-case realized profit in cents (`worst_case_payout - filled_stake`).
    pub realized_profit: Cents,
    /// Expected profit from the detected signal in cents.
    pub expected_profit: Cents,
    /// Slippage in cents (`expected_profit - realized_profit`).
    pub slippage: Cents,
    /// Aggregate fill ratio in basis points (`filled_stake * 10000 / requested_stake`).
    pub fill_ratio_bps: u32,
    /// Realized profit as basis points of filled stake.
    pub realized_profit_bps: i64,
}

impl ExecutionPnl {
    /// Compute the worst-case PnL from the per-leg fill results.
    ///
    /// For an n-leg arbitrage to be risk-free, every possible outcome must yield
    /// a payout that covers the total capital staked across *all* legs. If any
    /// leg failed to fill, the payout if the other outcome occurs is zero,
    /// which turns the trade into a directional loss.
    pub fn compute(legs: &[LegFillResult], requested_stake: Cents, expected_profit: Cents) -> Self {
        let mut filled_stake: Cents = 0;
        let mut total_fees: Cents = 0;

        for leg in legs {
            filled_stake = filled_stake.saturating_add(leg.filled_stake);
            total_fees = total_fees.saturating_add(leg.fee_paid);
        }

        // If no legs filled, everything is zero.
        if filled_stake <= 0 {
            return Self {
                requested_stake,
                filled_stake: 0,
                total_fees: 0,
                worst_case_payout: 0,
                realized_profit: 0,
                expected_profit,
                slippage: expected_profit,
                fill_ratio_bps: 0,
                realized_profit_bps: 0,
            };
        }

        // Determine payout for each outcome scenario.
        // If outcome i wins: we receive leg[i].net_payout.
        // If any leg had 0 fill, and its outcome occurs, payout is 0!
        let mut worst_payout: Cents = Cents::MAX;
        for leg in legs {
            worst_payout = worst_payout.min(leg.net_payout);
        }

        let realized_profit = worst_payout.saturating_sub(filled_stake);
        let slippage = expected_profit.saturating_sub(realized_profit);

        let fill_ratio_bps = if requested_stake > 0 {
            ((filled_stake as i128 * BPS) / (requested_stake as i128)) as u32
        } else {
            0
        };

        let realized_profit_bps = if filled_stake > 0 {
            ((realized_profit as i128 * BPS) / (filled_stake as i128)) as i64
        } else {
            0
        };

        Self {
            requested_stake,
            filled_stake,
            total_fees,
            worst_case_payout: worst_payout,
            realized_profit,
            expected_profit,
            slippage,
            fill_ratio_bps,
            realized_profit_bps,
        }
    }
}

/// Cumulative statistics for backtesting and paper trading simulation runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SimulationStats {
    /// Total detected signals evaluated.
    pub total_signals: u64,
    /// Signals for which capital was actually committed and an execution was
    /// attempted (i.e. `total_signals` minus `capital_short`).
    pub attempted: u64,
    /// Signals skipped outright because the bankroll had insufficient
    /// available capital to reserve the requested stake — no order was ever
    /// sent. See `Bankroll::reserve`.
    pub capital_short: u64,
    /// Clean fills with full hedge.
    pub clean_fills: u64,
    /// Partial fills with preserved hedge.
    pub proportional_fills: u64,
    /// Total phantom opportunities encountered.
    pub total_phantoms: u64,
    /// Broken leg occurrences (unhedged directional risk).
    pub broken_legs: u64,
    /// Cumulative stake requested in cents.
    pub total_requested_stake_cents: Cents,
    /// Cumulative stake filled in cents.
    pub total_filled_stake_cents: Cents,
    /// Cumulative exchange and stake fees paid in cents.
    pub total_fees_paid_cents: Cents,
    /// Cumulative worst-case payout received in cents.
    pub total_worst_case_payout_cents: Cents,
    /// Cumulative expected profit from detected signals in cents.
    pub total_expected_profit_cents: Cents,
    /// Cumulative realized profit in cents.
    pub total_realized_profit_cents: Cents,
    /// Cumulative execution slippage in cents.
    pub total_slippage_cents: Cents,
    /// Number of signals filled only by chasing at least one leg past its
    /// detected quote, under a [`crate::simulator::ChasePolicy`].
    pub chased_count: u64,
    /// Cumulative realized profit in cents from chased fills, in cents.
    ///
    /// A subset of `total_realized_profit_cents`, broken out so a chase
    /// policy's contribution (and risk) can be measured on its own.
    pub chased_profit_cents: Cents,
}

impl SimulationStats {
    /// Incorporate an execution result into the cumulative statistics.
    pub fn record(
        &mut self,
        pnl: &ExecutionPnl,
        is_clean: bool,
        is_partial: bool,
        is_phantom: bool,
        is_broken: bool,
    ) {
        self.total_signals += 1;
        self.attempted += 1;
        if is_clean {
            self.clean_fills += 1;
        } else if is_partial {
            self.proportional_fills += 1;
        }
        if is_phantom {
            self.total_phantoms += 1;
        }
        if is_broken {
            self.broken_legs += 1;
        }

        self.total_requested_stake_cents = self
            .total_requested_stake_cents
            .saturating_add(pnl.requested_stake);
        self.total_filled_stake_cents = self
            .total_filled_stake_cents
            .saturating_add(pnl.filled_stake);
        self.total_fees_paid_cents = self.total_fees_paid_cents.saturating_add(pnl.total_fees);
        self.total_worst_case_payout_cents = self
            .total_worst_case_payout_cents
            .saturating_add(pnl.worst_case_payout);
        self.total_expected_profit_cents = self
            .total_expected_profit_cents
            .saturating_add(pnl.expected_profit);
        self.total_realized_profit_cents = self
            .total_realized_profit_cents
            .saturating_add(pnl.realized_profit);
        self.total_slippage_cents = self.total_slippage_cents.saturating_add(pnl.slippage);
    }

    /// Record a signal that was skipped outright because the bankroll had
    /// insufficient available capital to reserve `requested_stake` at some
    /// leg's venue. No order was sent, so there is no [`ExecutionPnl`] to
    /// fold in: the signal counts toward `total_signals` and
    /// `total_requested_stake_cents` (capital that *would* have been needed)
    /// but not toward `attempted`, `total_filled_stake_cents`, or any profit
    /// field.
    pub fn record_capital_short(&mut self, requested_stake: Cents) {
        self.total_signals += 1;
        self.capital_short += 1;
        self.total_requested_stake_cents = self
            .total_requested_stake_cents
            .saturating_add(requested_stake);
    }

    /// Record that a signal was filled only by chasing at least one leg
    /// past its detected quote. Called in addition to, never instead of,
    /// [`SimulationStats::record`].
    pub fn record_chase(&mut self, realized_profit_cents: Cents) {
        self.chased_count += 1;
        self.chased_profit_cents = self
            .chased_profit_cents
            .saturating_add(realized_profit_cents);
    }

    /// Aggregate fill ratio in basis points (`total_filled_stake * 10000 / total_requested_stake`).
    #[inline]
    pub fn aggregate_fill_ratio_bps(&self) -> u32 {
        if self.total_requested_stake_cents <= 0 {
            return 0;
        }
        ((self.total_filled_stake_cents as i128 * BPS) / (self.total_requested_stake_cents as i128))
            as u32
    }

    /// Return on investment (ROI) in basis points (`total_realized_profit * 10000 / total_filled_stake`).
    #[inline]
    pub fn realized_roi_bps(&self) -> i64 {
        if self.total_filled_stake_cents <= 0 {
            return 0;
        }
        ((self.total_realized_profit_cents as i128 * BPS) / (self.total_filled_stake_cents as i128))
            as i64
    }

    /// Return on investment (ROI) in basis points, against every cent of
    /// capital *that would have needed to be committed* to attempt every
    /// detected signal (`total_realized_profit * 10000 / total_requested_stake`),
    /// including signals that never fired an order because the bankroll was
    /// capital-short.
    ///
    /// This diverges from [`Self::realized_roi_bps`] whenever
    /// `capital_short > 0`: `realized_roi_bps` divides by capital actually
    /// put to work (`total_filled_stake_cents`), so it flatters a bankroll
    /// that was too small to take every signal it found — the stake it
    /// couldn't afford to risk never counts against it. `attempted_roi_bps`
    /// uses the larger, more conservative denominator (all requested stake,
    /// affordable or not), per the "when in doubt, be pessimistic" rule.
    #[inline]
    pub fn attempted_roi_bps(&self) -> i64 {
        if self.total_requested_stake_cents <= 0 {
            return 0;
        }
        ((self.total_realized_profit_cents as i128 * BPS)
            / (self.total_requested_stake_cents as i128)) as i64
    }

    /// Phantom rate in basis points (`total_phantoms * 10000 / total_signals`).
    #[inline]
    pub fn phantom_rate_bps(&self) -> u32 {
        if self.total_signals == 0 {
            return 0;
        }
        ((self.total_phantoms as u128 * BPS as u128) / (self.total_signals as u128)) as u32
    }

    /// Total realized profit in dollars as `f64` (for reporting/display only).
    #[inline]
    pub fn realized_profit_dollars_f64(&self) -> f64 {
        self.total_realized_profit_cents as f64 / 100.0
    }

    /// Total staked in dollars as `f64` (for reporting/display only).
    #[inline]
    pub fn filled_stake_dollars_f64(&self) -> f64 {
        self.total_filled_stake_cents as f64 / 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::order::{LegFillStatus, UnfilledReason};
    use arbkit_core::{Fee, Prob};

    #[test]
    fn perfect_two_leg_fill_pnl() {
        let p50 = Prob::from_cents(50).unwrap();
        let leg0 = LegFillResult::compute_fill(
            0,
            0,
            50_000,
            50_000,
            p50,
            Fee::None,
            LegFillStatus::Filled,
            100,
        );
        let leg1 = LegFillResult::compute_fill(
            1,
            1,
            50_000,
            50_000,
            p50,
            Fee::None,
            LegFillStatus::Filled,
            100,
        );

        let pnl = ExecutionPnl::compute(&[leg0, leg1], 100_000, 2_000);
        assert_eq!(pnl.filled_stake, 100_000);
        assert_eq!(pnl.worst_case_payout, 100_000);
        assert_eq!(pnl.realized_profit, 0); // At 50c/50c, payout is exactly $1000
    }

    #[test]
    fn broken_leg_yields_worst_case_loss_of_filled_stake() {
        let p50 = Prob::from_cents(50).unwrap();
        // Leg 0 filled $500
        let leg0 = LegFillResult::compute_fill(
            0,
            0,
            50_000,
            50_000,
            p50,
            Fee::None,
            LegFillStatus::Filled,
            100,
        );
        // Leg 1 completely unfilled
        let leg1 = LegFillResult::unfilled(1, 1, 50_000, p50, UnfilledReason::BookStale, 100);

        let pnl = ExecutionPnl::compute(&[leg0, leg1], 100_000, 2_000);
        assert_eq!(pnl.filled_stake, 50_000);
        // If outcome 1 wins, payout is 0, so worst case payout is 0
        assert_eq!(pnl.worst_case_payout, 0);
        // Realized profit is 0 - 50_000 = -50_000 (total loss of filled leg)
        assert_eq!(pnl.realized_profit, -50_000);
        assert_eq!(pnl.realized_profit_bps, -10_000); // -100%
    }

    #[test]
    fn attempted_roi_diverges_from_realized_roi_with_capital_short_skips() {
        // 48c on venue 0, 50c on venue 1: $980 buys $1,000 of guaranteed
        // payout, a clean $20 (2_000-cent) profit on $980 (98_000-cent) of
        // filled stake.
        let p48 = Prob::from_cents(48).unwrap();
        let p50 = Prob::from_cents(50).unwrap();
        let leg0 = LegFillResult::compute_fill(
            0,
            0,
            48_000,
            48_000,
            p48,
            Fee::None,
            LegFillStatus::Filled,
            100,
        );
        let leg1 = LegFillResult::compute_fill(
            1,
            1,
            50_000,
            50_000,
            p50,
            Fee::None,
            LegFillStatus::Filled,
            100,
        );
        let pnl = ExecutionPnl::compute(&[leg0, leg1], 98_000, 2_000);
        assert_eq!(pnl.filled_stake, 98_000);
        assert_eq!(pnl.realized_profit, 2_000);

        let mut stats = SimulationStats::default();
        stats.record(&pnl, true, false, false, false);

        // Without any capital-short skips, both ROI figures agree (same
        // denominator: filled stake == requested stake here).
        assert_eq!(stats.realized_roi_bps(), stats.attempted_roi_bps());
        assert_eq!(stats.attempted, 1);
        assert_eq!(stats.capital_short, 0);

        // Now a signal requesting $2,000 (200_000 cents) of stake is skipped
        // outright because the bankroll couldn't fund it: no fill, no
        // profit, but the capital that *would* have been needed still shows
        // up in the "attempted" denominator.
        stats.record_capital_short(200_000);

        assert_eq!(stats.total_signals, 2);
        assert_eq!(stats.attempted, 1);
        assert_eq!(stats.capital_short, 1);

        // realized_roi_bps is unaffected by the skip: still 2_000 / 98_000.
        assert_eq!(stats.realized_roi_bps(), (2_000 * BPS / 98_000) as i64);
        // attempted_roi_bps divides the same 2_000-cent profit by all
        // 298_000 cents of capital that would have needed to be committed to
        // attempt both signals, so it is strictly smaller (more
        // conservative).
        assert_eq!(stats.attempted_roi_bps(), (2_000 * BPS / 298_000) as i64);
        assert!(stats.attempted_roi_bps() < stats.realized_roi_bps());
    }
}
