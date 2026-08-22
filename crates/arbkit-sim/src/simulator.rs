//! Paper trading simulator and backtesting execution engine.
//!
//! Simulates end-to-end execution of detected arbitrage opportunities against
//! resting liquidity, incorporating realistic wire delay, venue matching latency,
//! and queue front-running degradation.

use crate::MAX_SIM_LEGS;
use arbkit_core::{Allocation, Cents, Leg, OutcomeBook, Prob, Signal, PPM};

use crate::accounting::{ExecutionPnl, SimulationStats};
use crate::error::{Result, SimError};
use crate::latency::LatencyModel;
use crate::order::{LegFillResult, LegFillStatus, PartialFillReason, UnfilledReason};
use crate::phantom::{ArbExecutionClassification, PhantomReason, PhantomStats};

/// One basis point denominator (100% = 10,000 bps).
const BPS: u64 = 10_000;

/// Policy governing whether a leg whose price moved past the detected quote
/// may still be chased at its arrival price, rather than dropped outright.
///
/// Chasing is a directional-risk hazard if it is allowed one leg at a time:
/// filling the leg that got a good move while dropping the leg that got a
/// bad one turns a hedge into a naked bet. So the gate this policy describes
/// is always evaluated jointly, across every leg of the signal at once -
/// see [`Simulator::simulate_with_quotes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChasePolicy {
    /// Whether chasing is permitted at all. `false` reproduces the historical
    /// behavior: any leg whose price moved past its detected quote drops the
    /// entire signal, unfilled.
    pub enabled: bool,
    /// The largest adverse per-leg move, in basis points of the detected
    /// quote's price, that may still be chased. A leg that moved further
    /// than this against us fails the joint check no matter how profitable
    /// the rest of the signal looks.
    pub max_chase_bps: u32,
}

impl ChasePolicy {
    /// Chasing turned off - the historical, unconditional leg-drop behavior.
    pub const DISABLED: ChasePolicy = ChasePolicy {
        enabled: false,
        max_chase_bps: 0,
    };
}

impl Default for ChasePolicy {
    #[inline]
    fn default() -> Self {
        Self::DISABLED
    }
}

/// Simulator-wide execution policy.
///
/// Defaults reproduce the historical simulator behavior exactly: chasing is
/// disabled and signals never expire on their own (a stale book or a moved
/// price is still what ends the attempt).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SimConfig {
    /// Governs whether a moved leg may still be chased at its arrival price.
    pub chase: ChasePolicy,
    /// Time-to-live for a detected signal, in nanoseconds, measured from
    /// `detection_timestamp_ns` to the attempt timestamp (the latest
    /// per-leg arrival time already computed by the latency model). `0`
    /// means the signal never expires on TTL grounds alone.
    pub signal_ttl_ns: u64,
}

/// Complete report of a simulated arbitrage execution attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionReport {
    /// Timestamp at which the signal was detected, in nanoseconds.
    pub detection_timestamp_ns: u64,
    /// Execution classification (clean fill, partial, or phantom).
    pub classification: ArbExecutionClassification,
    /// Whether this fill was only achieved by chasing at least one leg past
    /// its detected quote under [`ChasePolicy`]. A chased execution can
    /// still classify as [`ArbExecutionClassification::CleanFill`]; this
    /// flag is what distinguishes it from a fill at the originally detected
    /// prices for reporting purposes.
    pub chased: bool,
    leg_results: [LegFillResult; MAX_SIM_LEGS],
    leg_count: u8,
    /// Detailed PnL accounting for this execution.
    pub pnl: ExecutionPnl,
}

impl ExecutionReport {
    /// Slice of individual leg execution results.
    #[inline]
    pub fn leg_results(&self) -> &[LegFillResult] {
        &self.leg_results[..self.leg_count as usize]
    }

    /// Whether the execution was fully filled and clean.
    #[inline]
    pub const fn is_clean(&self) -> bool {
        self.classification.is_clean()
    }

    /// Whether this execution was a phantom.
    #[inline]
    pub const fn is_phantom(&self) -> bool {
        self.classification.is_phantom()
    }

    /// Whether this execution suffered an unhedged broken leg.
    #[inline]
    pub const fn is_broken_leg(&self) -> bool {
        self.classification.is_broken_leg()
    }
}

/// Paper trading and backtesting simulator.
#[derive(Debug, Clone)]
pub struct Simulator {
    latency_model: LatencyModel,
    config: SimConfig,
    stats: SimulationStats,
    phantom_stats: PhantomStats,
}

impl Simulator {
    /// Create a new simulator with the given latency model and the default
    /// [`SimConfig`] (chasing disabled, signals never TTL-expire).
    pub fn new(latency_model: LatencyModel) -> Self {
        Self::with_config(latency_model, SimConfig::default())
    }

    /// Create a new simulator with an explicit execution policy.
    pub fn with_config(latency_model: LatencyModel, config: SimConfig) -> Self {
        Self {
            latency_model,
            config,
            stats: SimulationStats::default(),
            phantom_stats: PhantomStats::default(),
        }
    }

    /// Access the latency model.
    #[inline]
    pub fn latency_model(&self) -> &LatencyModel {
        &self.latency_model
    }

    /// Access the current execution policy.
    #[inline]
    pub fn config(&self) -> SimConfig {
        self.config
    }

    /// Replace the execution policy.
    #[inline]
    pub fn set_config(&mut self, config: SimConfig) {
        self.config = config;
    }

    /// Access cumulative simulation statistics.
    #[inline]
    pub fn stats(&self) -> &SimulationStats {
        &self.stats
    }

    /// Record a signal that was skipped because the caller's bankroll could
    /// not reserve the requested stake. Folds into this simulator's stats so
    /// the disposition funnel (`attempted` vs `capital_short`) stays in one
    /// place even when the capital gate lives outside the simulator.
    pub fn record_capital_short(&mut self, requested_stake: Cents) {
        self.stats.record_capital_short(requested_stake);
    }

    /// Access phantom arbitrage statistics.
    #[inline]
    pub fn phantom_stats(&self) -> &PhantomStats {
        &self.phantom_stats
    }

    /// Reset all recorded statistics.
    pub fn reset(&mut self) {
        self.stats = SimulationStats::default();
        self.phantom_stats = PhantomStats::default();
    }

    /// Simulate execution of a detected [`Signal`] against arrival order books.
    ///
    /// Evaluates wire and venue latencies for each venue, checks price and
    /// depth validity upon arrival, accounts for queue degradation, and tracks PnL.
    pub fn simulate_signal(
        &mut self,
        detection_timestamp_ns: u64,
        signal: &Signal,
        legs: &[Leg],
        arrival_books: &[OutcomeBook],
    ) -> Result<ExecutionReport> {
        let leg_count = legs.len();
        if !(2..=MAX_SIM_LEGS).contains(&leg_count) {
            return Err(SimError::InvalidLegCount(leg_count));
        }
        if arrival_books.len() != leg_count {
            return Err(SimError::InvalidLegCount(arrival_books.len()));
        }

        let allocs = signal.allocations();
        if allocs.len() != leg_count {
            return Err(SimError::InvalidLegCount(allocs.len()));
        }

        let dummy = LegFillResult::unfilled(0, 0, 0, Prob::CERTAIN, UnfilledReason::BookStale, 0);
        let mut leg_results = [dummy; MAX_SIM_LEGS];

        for (i, leg) in legs.iter().enumerate() {
            let alloc = allocs[i];
            let requested_stake = alloc.stake;
            if requested_stake <= 0 {
                return Err(SimError::ZeroStake(i));
            }

            let arrival_time_ns = self
                .latency_model
                .arrival_time_ns(detection_timestamp_ns, leg.venue);
            let book = &arrival_books[i];

            let fill_res = self.evaluate_leg(leg, requested_stake, book, arrival_time_ns);
            leg_results[i] = fill_res;
        }

        let pnl = ExecutionPnl::compute(
            &leg_results[..leg_count],
            signal.total_stake,
            signal.worst_case_profit,
        );

        let classification = self.classify_execution(legs, &leg_results[..leg_count], &pnl);

        self.phantom_stats.record(classification);
        self.stats.record(
            &pnl,
            classification.is_clean(),
            matches!(
                classification,
                ArbExecutionClassification::ProportionalPartialFill
            ),
            classification.is_phantom(),
            classification.is_broken_leg(),
        );

        Ok(ExecutionReport {
            detection_timestamp_ns,
            classification,
            chased: false,
            leg_results,
            leg_count: leg_count as u8,
            pnl,
        })
    }

    /// Simulate execution with explicit prices and depths.
    pub fn simulate_with_quotes(
        &mut self,
        detection_timestamp_ns: u64,
        signal: &Signal,
        legs: &[Leg],
        arrival_prices: &[Option<Prob>],
        arrival_depths: &[Cents],
    ) -> Result<ExecutionReport> {
        let leg_count = legs.len();
        if !(2..=MAX_SIM_LEGS).contains(&leg_count) {
            return Err(SimError::InvalidLegCount(leg_count));
        }
        if arrival_prices.len() != leg_count || arrival_depths.len() != leg_count {
            return Err(SimError::InvalidLegCount(arrival_prices.len()));
        }

        let allocs = signal.allocations();
        if allocs.len() != leg_count {
            return Err(SimError::InvalidLegCount(allocs.len()));
        }

        let dummy = LegFillResult::unfilled(0, 0, 0, Prob::CERTAIN, UnfilledReason::BookStale, 0);
        let mut leg_results = [dummy; MAX_SIM_LEGS];
        let mut arrival_times = [0u64; MAX_SIM_LEGS];

        for (i, leg) in legs.iter().enumerate() {
            let alloc = allocs[i];
            let requested_stake = alloc.stake;
            if requested_stake <= 0 {
                return Err(SimError::ZeroStake(i));
            }

            let arrival_time_ns = self
                .latency_model
                .arrival_time_ns(detection_timestamp_ns, leg.venue);
            arrival_times[i] = arrival_time_ns;
            let maybe_price = arrival_prices[i];
            let raw_depth = arrival_depths[i];

            let fill_res = match maybe_price {
                None => LegFillResult::unfilled(
                    leg.venue,
                    leg.outcome,
                    requested_stake,
                    leg.quoted,
                    UnfilledReason::PriceMoved {
                        expected: leg.quoted,
                        current: None,
                    },
                    arrival_time_ns,
                ),
                Some(current_price) if current_price > leg.quoted => {
                    // Larger Prob is worse price (smaller decimal payout)
                    LegFillResult::unfilled(
                        leg.venue,
                        leg.outcome,
                        requested_stake,
                        leg.quoted,
                        UnfilledReason::PriceMoved {
                            expected: leg.quoted,
                            current: Some(current_price),
                        },
                        arrival_time_ns,
                    )
                }
                Some(current_price) => {
                    let effective_depth = self.latency_model.effective_depth(leg.venue, raw_depth);

                    if effective_depth <= 0 {
                        LegFillResult::unfilled(
                            leg.venue,
                            leg.outcome,
                            requested_stake,
                            current_price,
                            UnfilledReason::DepthExhausted {
                                available: 0,
                                requested: requested_stake,
                            },
                            arrival_time_ns,
                        )
                    } else if effective_depth >= requested_stake {
                        LegFillResult::compute_fill(
                            leg.venue,
                            leg.outcome,
                            requested_stake,
                            requested_stake,
                            current_price,
                            leg.fee,
                            LegFillStatus::Filled,
                            arrival_time_ns,
                        )
                    } else {
                        let increment = leg.increment.max(1);
                        let fillable = (effective_depth / increment) * increment;
                        if fillable <= 0 {
                            LegFillResult::unfilled(
                                leg.venue,
                                leg.outcome,
                                requested_stake,
                                current_price,
                                UnfilledReason::IncrementConstraint,
                                arrival_time_ns,
                            )
                        } else {
                            LegFillResult::compute_fill(
                                leg.venue,
                                leg.outcome,
                                requested_stake,
                                fillable,
                                current_price,
                                leg.fee,
                                LegFillStatus::PartiallyFilled {
                                    filled_stake: fillable,
                                    unfilled_stake: requested_stake - fillable,
                                    reason: PartialFillReason::DepthDepleted,
                                },
                                arrival_time_ns,
                            )
                        }
                    }
                }
            };

            leg_results[i] = fill_res;
        }

        // A leg dropped for having moved past its detected quote is, absent
        // a chase policy, exactly what ends the attempt. When chasing is
        // enabled, re-evaluate the whole signal jointly at arrival prices
        // rather than leaving the drop in place - but only when at least
        // one leg actually needed it, so a signal that filled cleanly at
        // the original quotes is untouched.
        let mut chased = false;
        if self.config.chase.enabled {
            let any_leg_moved = (0..leg_count).any(|i| {
                matches!(
                    leg_results[i].status,
                    LegFillStatus::Unfilled(UnfilledReason::PriceMoved { .. })
                )
            });

            if any_leg_moved {
                let attempt_ts = arrival_times[..leg_count]
                    .iter()
                    .copied()
                    .max()
                    .unwrap_or(detection_timestamp_ns);
                let ttl = self.config.signal_ttl_ns;
                let ttl_ok = ttl == 0 || attempt_ts.saturating_sub(detection_timestamp_ns) <= ttl;

                if ttl_ok {
                    if let Some(chased_results) = self.try_chase(
                        legs,
                        allocs,
                        arrival_prices,
                        arrival_depths,
                        &arrival_times[..leg_count],
                    ) {
                        leg_results[..leg_count].copy_from_slice(&chased_results[..leg_count]);
                        chased = true;
                    }
                }
            }
        }

        let pnl = ExecutionPnl::compute(
            &leg_results[..leg_count],
            signal.total_stake,
            signal.worst_case_profit,
        );

        let classification = self.classify_execution(legs, &leg_results[..leg_count], &pnl);

        self.phantom_stats.record(classification);
        self.stats.record(
            &pnl,
            classification.is_clean(),
            matches!(
                classification,
                ArbExecutionClassification::ProportionalPartialFill
            ),
            classification.is_phantom(),
            classification.is_broken_leg(),
        );
        if chased {
            self.stats.record_chase(pnl.realized_profit);
        }

        Ok(ExecutionReport {
            detection_timestamp_ns,
            classification,
            chased,
            leg_results,
            leg_count: leg_count as u8,
            pnl,
        })
    }

    /// Joint re-check and fill for a chased signal.
    ///
    /// All-or-nothing: this either returns fill results for every leg
    /// (which may themselves be partial fills or, if depth vanished too,
    /// unfilled) or `None`, in which case the caller keeps whatever the
    /// original per-leg evaluation produced. It never fills a subset of
    /// legs and drops the rest - that is exactly the unhedged directional
    /// bet the all-or-nothing invariant exists to prevent.
    ///
    /// The gate has two parts, both evaluated across every leg before any
    /// leg is touched:
    /// 1. Every leg must still have a quote, and none may have moved
    ///    against us by more than [`ChasePolicy::max_chase_bps`].
    /// 2. The fee-adjusted overround recomputed from arrival prices must
    ///    still be under parity - a chase never fills into a trade that
    ///    arrival prices show is no longer profitable.
    fn try_chase(
        &self,
        legs: &[Leg],
        allocs: &[Allocation],
        arrival_prices: &[Option<Prob>],
        arrival_depths: &[Cents],
        arrival_times: &[u64],
    ) -> Option<[LegFillResult; MAX_SIM_LEGS]> {
        let leg_count = legs.len();
        let max_chase_bps = u64::from(self.config.chase.max_chase_bps);

        let mut arrival_overround_ppm: u64 = 0;
        for (i, leg) in legs.iter().enumerate() {
            let price = arrival_prices[i]?;

            if price > leg.quoted && adverse_move_bps(leg.quoted, price) > max_chase_bps {
                return None;
            }

            arrival_overround_ppm += u64::from(leg.fee.effective(price).ppm());
        }

        if arrival_overround_ppm >= u64::from(PPM) {
            return None;
        }

        let dummy = LegFillResult::unfilled(0, 0, 0, Prob::CERTAIN, UnfilledReason::BookStale, 0);
        let mut results = [dummy; MAX_SIM_LEGS];

        for i in 0..leg_count {
            let leg = &legs[i];
            let requested_stake = allocs[i].stake;
            // Presence already checked above.
            let price = arrival_prices[i].expect("price checked in gate above");
            let raw_depth = arrival_depths[i];
            let arrival_time_ns = arrival_times[i];
            let effective_depth = self.latency_model.effective_depth(leg.venue, raw_depth);

            results[i] = if effective_depth <= 0 {
                LegFillResult::unfilled(
                    leg.venue,
                    leg.outcome,
                    requested_stake,
                    price,
                    UnfilledReason::DepthExhausted {
                        available: 0,
                        requested: requested_stake,
                    },
                    arrival_time_ns,
                )
            } else if effective_depth >= requested_stake {
                LegFillResult::compute_fill(
                    leg.venue,
                    leg.outcome,
                    requested_stake,
                    requested_stake,
                    price,
                    leg.fee,
                    LegFillStatus::Filled,
                    arrival_time_ns,
                )
            } else {
                let increment = leg.increment.max(1);
                let fillable = (effective_depth / increment) * increment;
                if fillable <= 0 {
                    LegFillResult::unfilled(
                        leg.venue,
                        leg.outcome,
                        requested_stake,
                        price,
                        UnfilledReason::IncrementConstraint,
                        arrival_time_ns,
                    )
                } else {
                    LegFillResult::compute_fill(
                        leg.venue,
                        leg.outcome,
                        requested_stake,
                        fillable,
                        price,
                        leg.fee,
                        LegFillStatus::PartiallyFilled {
                            filled_stake: fillable,
                            unfilled_stake: requested_stake - fillable,
                            reason: PartialFillReason::DepthDepleted,
                        },
                        arrival_time_ns,
                    )
                }
            };
        }

        Some(results)
    }

    fn evaluate_leg(
        &self,
        leg: &Leg,
        requested_stake: Cents,
        book: &OutcomeBook,
        arrival_time_ns: u64,
    ) -> LegFillResult {
        if book.is_stale() {
            return LegFillResult::unfilled(
                leg.venue,
                leg.outcome,
                requested_stake,
                leg.quoted,
                UnfilledReason::BookStale,
                arrival_time_ns,
            );
        }

        let best_level = match book.best() {
            Some(lvl) => lvl,
            None => {
                return LegFillResult::unfilled(
                    leg.venue,
                    leg.outcome,
                    requested_stake,
                    leg.quoted,
                    UnfilledReason::PriceMoved {
                        expected: leg.quoted,
                        current: None,
                    },
                    arrival_time_ns,
                );
            }
        };

        // If best resting price is shorter (worse) than detected quote
        if best_level.price > leg.quoted {
            return LegFillResult::unfilled(
                leg.venue,
                leg.outcome,
                requested_stake,
                leg.quoted,
                UnfilledReason::PriceMoved {
                    expected: leg.quoted,
                    current: Some(best_level.price),
                },
                arrival_time_ns,
            );
        }

        // Available depth at or better than quoted price
        let raw_depth = book.depth_to(leg.quoted);
        let effective_depth = self.latency_model.effective_depth(leg.venue, raw_depth);

        if effective_depth <= 0 {
            return LegFillResult::unfilled(
                leg.venue,
                leg.outcome,
                requested_stake,
                best_level.price,
                UnfilledReason::DepthExhausted {
                    available: 0,
                    requested: requested_stake,
                },
                arrival_time_ns,
            );
        }

        if effective_depth >= requested_stake {
            LegFillResult::compute_fill(
                leg.venue,
                leg.outcome,
                requested_stake,
                requested_stake,
                best_level.price,
                leg.fee,
                LegFillStatus::Filled,
                arrival_time_ns,
            )
        } else {
            let increment = leg.increment.max(1);
            let fillable = (effective_depth / increment) * increment;
            if fillable <= 0 {
                LegFillResult::unfilled(
                    leg.venue,
                    leg.outcome,
                    requested_stake,
                    best_level.price,
                    UnfilledReason::IncrementConstraint,
                    arrival_time_ns,
                )
            } else {
                LegFillResult::compute_fill(
                    leg.venue,
                    leg.outcome,
                    requested_stake,
                    fillable,
                    best_level.price,
                    leg.fee,
                    LegFillStatus::PartiallyFilled {
                        filled_stake: fillable,
                        unfilled_stake: requested_stake - fillable,
                        reason: PartialFillReason::DepthDepleted,
                    },
                    arrival_time_ns,
                )
            }
        }
    }

    fn classify_execution(
        &self,
        legs: &[Leg],
        leg_results: &[LegFillResult],
        pnl: &ExecutionPnl,
    ) -> ArbExecutionClassification {
        let all_filled = leg_results.iter().all(|r| r.is_fully_filled());
        if all_filled {
            if pnl.realized_profit > 0 {
                return ArbExecutionClassification::CleanFill;
            } else {
                return ArbExecutionClassification::Phantom(PhantomReason::UnprofitableAfterCosts);
            }
        }

        let filled_count = leg_results.iter().filter(|r| r.filled_stake > 0).count();
        let unfilled_count = leg_results.iter().filter(|r| r.filled_stake == 0).count();

        // Broken leg: at least one leg filled with stake > 0, and at least one leg completely unfilled (0 fill)
        if filled_count > 0 && unfilled_count > 0 {
            let mut filled_venue = 0;
            let mut failed_venue = 0;
            for (i, r) in leg_results.iter().enumerate() {
                if r.filled_stake > 0 {
                    filled_venue = legs[i].venue;
                } else {
                    failed_venue = legs[i].venue;
                }
            }
            return ArbExecutionClassification::Phantom(PhantomReason::BrokenLeg {
                filled_venue,
                failed_venue,
            });
        }

        // All legs were partially filled
        if filled_count == leg_results.len() {
            // Check if hedge is preserved (worst case payout covers filled stake)
            if pnl.realized_profit > 0 {
                return ArbExecutionClassification::ProportionalPartialFill;
            } else {
                return ArbExecutionClassification::Phantom(PhantomReason::AsymmetricFill);
            }
        }

        // All legs failed (0 fills across all legs)
        for (i, r) in leg_results.iter().enumerate() {
            if let LegFillStatus::Unfilled(reason) = r.status {
                return match reason {
                    UnfilledReason::PriceMoved { .. } => {
                        ArbExecutionClassification::Phantom(PhantomReason::PriceMoved {
                            venue: legs[i].venue,
                        })
                    }
                    UnfilledReason::DepthExhausted { .. } => {
                        ArbExecutionClassification::Phantom(PhantomReason::DepthExhausted {
                            venue: legs[i].venue,
                        })
                    }
                    UnfilledReason::BookStale => {
                        ArbExecutionClassification::Phantom(PhantomReason::BookStale {
                            venue: legs[i].venue,
                        })
                    }
                    UnfilledReason::IncrementConstraint => {
                        ArbExecutionClassification::Phantom(PhantomReason::DepthExhausted {
                            venue: legs[i].venue,
                        })
                    }
                };
            }
        }

        ArbExecutionClassification::Phantom(PhantomReason::UnprofitableAfterCosts)
    }
}

impl Default for Simulator {
    fn default() -> Self {
        Self::new(LatencyModel::default())
    }
}

/// Adverse price movement of `current` against `quoted`, in basis points of
/// the quoted price. Callers only invoke this when `current > quoted` (a
/// worse price - see [`Prob`]'s ordering), so the subtraction never
/// underflows.
///
/// Rounds up rather than down: per the workspace-wide pessimistic-rounding
/// rule, a move we are unsure about should read as further against us, not
/// closer to passing the [`ChasePolicy::max_chase_bps`] gate than it really
/// is.
#[inline]
fn adverse_move_bps(quoted: Prob, current: Prob) -> u64 {
    let quoted_ppm = u64::from(quoted.ppm());
    let diff_ppm = u64::from(current.ppm() - quoted.ppm());
    (diff_ppm * BPS).div_ceil(quoted_ppm)
}
