//! Fast market-level quote aggregator and arbitrage detector invocation.
//!
//! Aggregates quotes across venues for all outcomes of a given market, applies
//! venue-specific fees, increments, and transit-survival depth discounts, line-
//! shops across every retained book level, constructs the fixed-size [`BookLeg`]
//! array, and invokes [`arbkit_core::detect_book`].
//!
//! Two things happen before detection that defend realized PnL:
//!
//! * **Depth is discounted before sizing** (B1): each level's resting size is
//!   passed through [`DepthDiscount`] so a signal only ever requests stake that
//!   survives the trip to the venue. Sizing against displayed depth and
//!   discounting at fill time was the structural source of partial fills.
//! * **Outcomes are line-shopped across venues** (B2): one side of the hedge
//!   may be split across several venues' books. Coverage stays fair — every
//!   outcome gets its best quote first — and then remaining chunk slots fill
//!   in globally best fee-adjusted order, capped at [`MAX_CHUNKS`].

use crate::error::Result;
use crate::slab::{EngineSlab, MarketConfig, MAX_OUTCOMES, MAX_VENUES};
use arbkit_core::arb::{detect_book, BookLeg, Leg, MAX_CHUNKS, MAX_LEVELS_PER_LEG};
use arbkit_core::book::{Cents, Level, MarketId, OutcomeId, VenueId};
use arbkit_core::{DepthDiscount, Fee, Prob, Signal};

/// Every `(outcome, venue, level)` triple the slab can hold.
///
/// The compile-time bound the hot-path rules require: candidate collection is
/// exactly this many iterations, and global selection is at most
/// [`MAX_CHUNKS`] sweeps over this array (~4k comparisons worst case).
const MAX_CHUNK_CANDIDATES: usize = MAX_OUTCOMES * MAX_VENUES * MAX_LEVELS_PER_LEG;

/// One quoted `(outcome, venue, level)` triple worth considering for the plan.
#[derive(Debug, Clone, Copy)]
struct ChunkCandidate {
    /// Outcome the chunk backs.
    outcome: usize,
    /// Venue quoting the chunk.
    venue: usize,
    /// Quoted price, before fees.
    price: Prob,
    /// Fee-adjusted implied probability — smaller is a better (longer) quote.
    eff_ppm: u32,
    /// Raw resting size at this level, as the book holds it.
    raw_size: Cents,
    /// Transit-discounted share of [`Self::raw_size`] the venue will
    /// realistically still hold — what detection is allowed to size against.
    size: Cents,
}

/// The empty level stamped into unused `BookLeg.levels` slots.
///
/// A zero size can never absorb stake, so unused slots are inert without any
/// branching on `n_levels` beyond the length the detector already checks.
const EMPTY_LEVEL: Level = Level {
    price: Prob::CERTAIN,
    size: 0,
};

/// The inert `BookLeg` stamped into unused leg slots.
const EMPTY_LEG: BookLeg = BookLeg {
    venue: 0,
    outcome: 0,
    fee: Fee::None,
    increment: 1,
    levels: [EMPTY_LEVEL; MAX_LEVELS_PER_LEG],
    n_levels: 0,
};

/// Market aggregator for zero-allocation quote evaluation.
pub struct Aggregator;

impl Aggregator {
    /// Evaluates all outcomes for `market_id` across all active venues and runs detection.
    ///
    /// Collects every live level from every venue, discounts sizes by the
    /// venue's survival rate, then builds a plan input of at most
    /// [`MAX_CHUNKS`] chunks: first the single best fee-adjusted quote per
    /// outcome (fair coverage — the all-or-nothing rule needs every side
    /// present), then the globally best remaining quotes regardless of
    /// outcome or venue, so deep books on cheap venues can back one side of
    /// the hedge across many lines.
    ///
    /// Returns `Ok(Some((Signal, plan legs, plan length)))` if a tradeable
    /// arbitrage exists — the legs slice index-aligns with the signal's
    /// allocations so downstream consumers can rebuild exact execution
    /// legs — `Ok(None)` if no edge or incomplete quotes exist, or `Err` on
    /// core domain errors.
    #[inline]
    pub fn evaluate_market(
        slab: &EngineSlab,
        market_id: MarketId,
    ) -> Result<Option<(Signal, [Leg; MAX_CHUNKS], u8)>> {
        let config = match slab.get_config(market_id) {
            Some(c) if c.active => c,
            _ => return Ok(None),
        };

        let outcome_count = config.outcome_count as usize;
        let mut candidates = [ChunkCandidate {
            outcome: 0,
            venue: 0,
            price: Prob::CERTAIN,
            eff_ppm: u32::MAX,
            raw_size: 0,
            size: 0,
        }; MAX_CHUNK_CANDIDATES];
        let mut n_candidates = 0usize;

        for outcome in 0..outcome_count {
            let outcome_id = outcome as OutcomeId;
            for venue in 0..MAX_VENUES {
                let Some(book) = slab.get_book(market_id, outcome_id, venue as VenueId) else {
                    continue;
                };
                // levels() yields nothing while stale: a lost sequence takes
                // the whole venue-outcome out of service, per the repo rule.
                let discount = DepthDiscount {
                    survival_bps: config.venue_survival_bps[venue],
                };
                let fee = config.venue_fees[venue];
                for level in book.levels() {
                    // Size against what survives transit, not what is
                    // displayed; a floored-to-zero level is no chunk at all.
                    // The raw size travels alongside: the simulator applies
                    // its own identical survival discount to raw arrival
                    // depth, and applying it twice would understate fills.
                    let size = discount.discounted(level.size);
                    if size <= 0 {
                        continue;
                    }
                    let eff_ppm = fee.effective(level.price).ppm();
                    candidates[n_candidates] = ChunkCandidate {
                        outcome,
                        venue,
                        price: level.price,
                        eff_ppm,
                        raw_size: level.size,
                        size,
                    };
                    n_candidates += 1;
                }
            }
        }

        // Fair coverage: one best chunk per outcome, or no arb — a market
        // missing a quote on any outcome cannot be hedged (invariant 7).
        let mut chosen = [false; MAX_CHUNK_CANDIDATES];
        let mut plan = [0usize; MAX_CHUNKS];
        let mut n_plan = 0usize;

        for outcome in 0..outcome_count {
            let mut best: Option<usize> = None;
            for (i, candidate) in candidates[..n_candidates].iter().enumerate() {
                if candidate.outcome == outcome
                    && best.is_none_or(|b| candidate.eff_ppm < candidates[b].eff_ppm)
                {
                    best = Some(i);
                }
            }
            let Some(i) = best else {
                return Ok(None);
            };
            chosen[i] = true;
            plan[n_plan] = i;
            n_plan += 1;
        }

        // Line shopping: fill the remaining chunk slots in globally best
        // fee-adjusted order. At most MAX_CHUNKS sweeps over the candidate
        // array; ties keep the earliest-scanned candidate, which keeps the
        // result deterministic for a given slab state.
        while n_plan < MAX_CHUNKS {
            let mut best: Option<usize> = None;
            for (i, candidate) in candidates[..n_candidates].iter().enumerate() {
                if !chosen[i] && best.is_none_or(|b| candidate.eff_ppm < candidates[b].eff_ppm) {
                    best = Some(i);
                }
            }
            let Some(i) = best else { break };
            chosen[i] = true;
            plan[n_plan] = i;
            n_plan += 1;
        }

        // Two detection inputs, same pessimistic detector:
        //
        // * the shopped plan — every selected chunk across venues and levels;
        // * the single-best plan — just the fair-coverage prefix, i.e. what
        //   a one-quote-per-outcome aggregator would have built.
        //
        // The payout-target search is a bounded heuristic, not an exact
        // optimizer: enlarging the chunk set can perturb which local optimum
        // it lands on (observed off-by-one-cent differences), even though
        // the shopped feasible set strictly contains the single-best one.
        // Running both and reporting the better signal restores the
        // monotonicity guarantee at the module boundary — every reported
        // number is still recomputed pessimistically by `detect_book` from
        // real chunks; we only choose which feasible plan to keep.
        let mut legs = [EMPTY_LEG; MAX_CHUNKS];
        for (leg_slot, &candidate_index) in legs.iter_mut().zip(plan.iter()).take(n_plan) {
            *leg_slot = Self::book_leg_from_candidate(config, &candidates[candidate_index]);
        }

        let mut single_best_legs = [EMPTY_LEG; MAX_CHUNKS];
        for (leg_slot, &candidate_index) in single_best_legs
            .iter_mut()
            .zip(plan.iter())
            .take(outcome_count)
        {
            *leg_slot = Self::book_leg_from_candidate(config, &candidates[candidate_index]);
        }

        let shopped = detect_book(&legs[..n_plan], config.budget)?;
        let single = detect_book(&single_best_legs[..outcome_count], config.budget)?;

        let prefer_single = match (&shopped, &single) {
            (Some(s), Some(t)) => t.worst_case_profit > s.worst_case_profit,
            (None, Some(_)) => true,
            _ => false,
        };
        let signal = if prefer_single {
            single.or(shopped)
        } else {
            shopped.or(single)
        };
        let Some(signal) = signal else {
            return Ok(None);
        };

        // The execution plan mirrors the *signal's* allocations one-to-one:
        // allocation i staked into detection-input leg `alloc.leg`, so the
        // consumer-side plan re-derives each execution leg from that chunk.
        // This keeps the execution plan index-aligned with
        // `Signal::allocations`, which is exactly what downstream simulators
        // iterate.
        let mut execution_plan = [Leg {
            venue: 0 as VenueId,
            outcome: 0 as OutcomeId,
            quoted: Prob::CERTAIN,
            fee: Fee::None,
            capacity: 0 as Cents,
            increment: 1 as Cents,
        }; MAX_CHUNKS];
        // `alloc.leg` indexes the detection input; both detection inputs
        // were laid out from the shared `plan` candidate-index array, so the
        // chunk behind any allocation is `plan[alloc.leg]`.
        for (i, alloc) in signal.allocations().iter().enumerate() {
            let candidate = &candidates[plan[alloc.leg]];
            execution_plan[i] = Leg {
                venue: candidate.venue as VenueId,
                outcome: candidate.outcome as OutcomeId,
                quoted: candidate.price,
                fee: config.venue_fees[candidate.venue],
                // Raw resting size: the consumer's fill model owns the
                // survival discount and must apply it exactly once.
                capacity: candidate.raw_size,
                increment: config.venue_increments[candidate.venue],
            };
        }
        let plan_len = signal.allocations().len() as u8;
        Ok(Some((signal, execution_plan, plan_len)))
    }

    /// Builds the one-level detection leg for a selected chunk candidate.
    #[inline]
    fn book_leg_from_candidate(config: &MarketConfig, candidate: &ChunkCandidate) -> BookLeg {
        let mut levels = [EMPTY_LEVEL; MAX_LEVELS_PER_LEG];
        levels[0] = Level {
            price: candidate.price,
            size: candidate.size,
        };
        BookLeg {
            venue: candidate.venue as VenueId,
            outcome: candidate.outcome as OutcomeId,
            fee: config.venue_fees[candidate.venue],
            increment: config.venue_increments[candidate.venue],
            levels,
            n_levels: 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slab::MarketConfig;
    use arbkit_core::book::Level;
    use arbkit_core::Fee;

    #[test]
    fn test_aggregator_finds_arbitrage() {
        let mut slab = EngineSlab::new(4);
        let mut config = MarketConfig {
            active: true,
            outcome_count: 2,
            budget: 100_000,
            ..Default::default()
        };
        config.venue_fees[0] = Fee::None;
        config.venue_fees[1] = Fee::None;
        config.venue_increments[0] = 1;
        config.venue_increments[1] = 1;

        slab.register_market(0, config).unwrap();

        // Venue 0 quotes outcome 0 at 48c
        let book0 = slab.get_book_mut(0, 0, 0).unwrap();
        book0.apply_snapshot(
            &[Level {
                price: Prob::from_cents(48).unwrap(),
                size: 50_000,
            }],
            1,
        );

        // Venue 1 quotes outcome 1 at 50c
        let book1 = slab.get_book_mut(0, 1, 1).unwrap();
        book1.apply_snapshot(
            &[Level {
                price: Prob::from_cents(50).unwrap(),
                size: 50_000,
            }],
            1,
        );

        let (signal, _, plan_len) = Aggregator::evaluate_market(&slab, 0).unwrap().unwrap();
        assert_eq!(plan_len, 2);
        assert_eq!(signal.profit_bps, 204);
    }

    #[test]
    fn test_aggregator_missing_outcome_returns_none() {
        let mut slab = EngineSlab::new(4);
        let config = MarketConfig {
            active: true,
            outcome_count: 2,
            ..Default::default()
        };
        slab.register_market(0, config).unwrap();

        // Only populate outcome 0
        let book0 = slab.get_book_mut(0, 0, 0).unwrap();
        book0.apply_snapshot(
            &[Level {
                price: Prob::from_cents(48).unwrap(),
                size: 50_000,
            }],
            1,
        );

        let result = Aggregator::evaluate_market(&slab, 0).unwrap();
        assert!(result.is_none());
    }

    /// Detection sizes against transit-surviving depth: halving each venue's
    /// survival rate must shrink (never grow) the staked plan, while the
    /// default `10_000` bps keeps the undiscounted answer bit-identical.
    #[test]
    fn test_aggregator_sizes_against_discounted_depth() {
        fn slab_with_survival(survival_bps: u32) -> EngineSlab {
            let mut slab = EngineSlab::new(4);
            let mut config = MarketConfig {
                active: true,
                outcome_count: 2,
                budget: 1_000_000,
                ..Default::default()
            };
            config.venue_survival_bps[0] = survival_bps;
            config.venue_survival_bps[1] = survival_bps;
            slab.register_market(0, config).unwrap();

            // Thin books: depth, not budget, is what caps the plan.
            let book0 = slab.get_book_mut(0, 0, 0).unwrap();
            book0.apply_snapshot(
                &[Level {
                    price: Prob::from_cents(48).unwrap(),
                    size: 4_000,
                }],
                1,
            );
            let book1 = slab.get_book_mut(0, 1, 1).unwrap();
            book1.apply_snapshot(
                &[Level {
                    price: Prob::from_cents(50).unwrap(),
                    size: 4_000,
                }],
                1,
            );
            slab
        }

        let (raw, _, _) = Aggregator::evaluate_market(&slab_with_survival(10_000), 0)
            .unwrap()
            .expect("undiscounted market is an arbitrage");
        let (half, _, _) = Aggregator::evaluate_market(&slab_with_survival(5_000), 0)
            .unwrap()
            .expect("discounted market stays an arbitrage");

        assert!(half.total_stake < raw.total_stake);
        // The floored discount of every level exactly halves the plan here.
        assert_eq!(half.total_stake * 2, raw.total_stake);
        assert!(half.worst_case_profit <= raw.worst_case_profit);

        let (none, _, _) = Aggregator::evaluate_market(&slab_with_survival(10_000), 0)
            .unwrap()
            .unwrap();
        assert_eq!(none, raw);
    }

    /// Line shopping's reason to exist, stated as a strict improvement: on
    /// these books, the exact single-best-quote-per-outcome plan a B1-style
    /// aggregator would build detects **nothing**, while shopping across
    /// every level detects a real arbitrage.
    ///
    /// The trap is a dust top-of-book: outcome 0's best price anywhere is
    /// venue 0's 44¢ — but its resting size sits below venue 0's 100-cent
    /// tradeable increment, so no plan can stake into it. A selector that
    /// carries only the best quote per outcome hands the detector that
    /// unusable leg and detection dies with it. Shopping keeps the worse
    /// 48¢ line from venue 1 alive as a fallback chunk and the hedge
    /// completes at 48 + 51 = 99¢.
    #[test]
    fn test_aggregator_strictly_improves_over_single_best_selection() {
        let mut slab = EngineSlab::new(4);
        let mut config = MarketConfig {
            active: true,
            outcome_count: 2,
            budget: 100_000,
            ..Default::default()
        };
        config.venue_increments[0] = 100;
        config.venue_increments[1] = 1;
        slab.register_market(0, config).unwrap();

        // Outcome 0: a dust 44¢ top on venue 0 (unusable: 80 < one 100¢
        // contract), real depth only on venue 1's worse 48¢ line.
        slab.get_book_mut(0, 0, 0).unwrap().apply_snapshot(
            &[Level {
                price: Prob::from_cents(44).unwrap(),
                size: 80,
            }],
            1,
        );
        slab.get_book_mut(0, 0, 1).unwrap().apply_snapshot(
            &[Level {
                price: Prob::from_cents(48).unwrap(),
                size: 40_000,
            }],
            1,
        );

        // Outcome 1: only venue 1 quotes it, 51¢.
        slab.get_book_mut(0, 1, 1).unwrap().apply_snapshot(
            &[Level {
                price: Prob::from_cents(51).unwrap(),
                size: 40_000,
            }],
            1,
        );

        // First, prove the counterfactual: the single-best-per-outcome
        // selector takes venue 0's 44¢ dust for outcome 0 (best price
        // anywhere), and that plan detects nothing — the dust leg cannot
        // absorb one increment, so the outcome group is infeasible.
        let single_best_plan = [
            BookLeg {
                venue: 0,
                outcome: 0,
                fee: Fee::None,
                increment: 100,
                levels: [
                    Level {
                        price: Prob::from_cents(44).unwrap(),
                        size: 80,
                    },
                    EMPTY_LEVEL,
                    EMPTY_LEVEL,
                    EMPTY_LEVEL,
                    EMPTY_LEVEL,
                    EMPTY_LEVEL,
                    EMPTY_LEVEL,
                    EMPTY_LEVEL,
                ],
                n_levels: 1,
            },
            BookLeg {
                venue: 1,
                outcome: 1,
                fee: Fee::None,
                increment: 1,
                levels: [
                    Level {
                        price: Prob::from_cents(51).unwrap(),
                        size: 40_000,
                    },
                    EMPTY_LEVEL,
                    EMPTY_LEVEL,
                    EMPTY_LEVEL,
                    EMPTY_LEVEL,
                    EMPTY_LEVEL,
                    EMPTY_LEVEL,
                    EMPTY_LEVEL,
                ],
                n_levels: 1,
            },
        ];
        let single_best =
            detect_book(&single_best_plan, 100_000).expect("well-formed input stays total");
        assert!(
            single_best.is_none(),
            "single-best selection must fail on the dust top"
        );

        // Now the shipped behavior: shopping past the dust finds the hedge.
        let (signal, _, _) = Aggregator::evaluate_market(&slab, 0)
            .unwrap()
            .expect("shopping past the unusable top must complete the arbitrage");
        assert!(signal.worst_case_profit > 0);

        // And no allocation may have staked into the dust chunk. Candidates
        // scan outcome-major, venue-minor, so the dust is candidate 0 and
        // lands on leg 0; whatever else the detector does, leg 0 cannot
        // absorb a single increment and must receive nothing.
        for allocation in signal.allocations() {
            assert_ne!(
                allocation.leg, 0,
                "dust chunk received {} cents of stake",
                allocation.stake
            );
        }
    }

    /// The chunk cap is hard: however rich the books are, the plan handed to
    /// the detector never exceeds `MAX_CHUNKS` legs.
    #[test]
    fn test_aggregator_respects_chunk_cap() {
        let mut slab = EngineSlab::new(4);
        let config = MarketConfig {
            active: true,
            outcome_count: 2,
            budget: 10_000_000,
            ..Default::default()
        };
        slab.register_market(0, config).unwrap();

        // Two outcomes × two venues × eight full levels = 32 candidates,
        // double the cap. Levels must descend in quality so every level is
        // a plausible take.
        for outcome in 0..2u32 {
            for venue in 0..2u16 {
                let base = 45 + outcome * 3;
                let levels: Vec<Level> = (0..MAX_LEVELS_PER_LEG as u32)
                    .map(|i| Level {
                        price: Prob::from_cents(base + i).unwrap(),
                        size: 10_000 - i as i64 * 100,
                    })
                    .collect();
                slab.get_book_mut(0, outcome, venue)
                    .unwrap()
                    .apply_snapshot(&levels, 1);
            }
        }

        // Must not panic, must stay total, and any signal it finds cannot
        // carry more allocations than the cap allows.
        if let Ok(Some((signal, _, plan_len))) = Aggregator::evaluate_market(&slab, 0) {
            assert!((plan_len as usize) <= MAX_CHUNKS);
            assert!(signal.allocations().len() <= MAX_CHUNKS);
        }
    }

    /// Whatever the books look like, the plan never stakes more into an
    /// outcome than the combined discounted capacity of the chunks it was
    /// given for that outcome.
    #[test]
    fn test_aggregate_stake_never_exceeds_combined_capacity() {
        let mut slab = EngineSlab::new(4);
        let config = MarketConfig {
            active: true,
            outcome_count: 2,
            budget: 500_000,
            venue_survival_bps: [5_000, 5_000, 10_000, 10_000, 10_000, 10_000, 10_000, 10_000],
            ..Default::default()
        };
        slab.register_market(0, config).unwrap();

        slab.get_book_mut(0, 0, 0).unwrap().apply_snapshot(
            &[
                Level {
                    price: Prob::from_cents(46).unwrap(),
                    size: 40_000,
                },
                Level {
                    price: Prob::from_cents(49).unwrap(),
                    size: 30_000,
                },
            ],
            1,
        );
        slab.get_book_mut(0, 1, 1).unwrap().apply_snapshot(
            &[
                Level {
                    price: Prob::from_cents(50).unwrap(),
                    size: 25_000,
                },
                Level {
                    price: Prob::from_cents(53).unwrap(),
                    size: 35_000,
                },
            ],
            1,
        );

        let (signal, _, _) = Aggregator::evaluate_market(&slab, 0)
            .unwrap()
            .expect("deep discounted books complete this arbitrage");

        // Capacity per outcome: 50% of (40k + 30k) and 50% of (25k + 35k).
        assert!(
            signal.total_stake <= 35_000 + 30_000,
            "staked {} exceeds combined discounted capacity",
            signal.total_stake
        );
    }
}

#[cfg(test)]
mod monotonicity_probe {
    //! Aggregator-level improvement monotonicity: shopping's plan must never
    //! report less pessimistic profit than a single-best-quote selector on
    //! identical slab state.
    use super::*;
    use crate::slab::MarketConfig;
    use arbkit_core::Fee;

    fn single_best_plan(slab: &EngineSlab, market_id: MarketId) -> Option<(Vec<BookLeg>, i64)> {
        let config = slab.get_config(market_id)?;
        if !config.active {
            return None;
        }
        let outcome_count = config.outcome_count as usize;
        let mut out = Vec::new();
        for outcome in 0..outcome_count {
            let mut best: Option<(usize, Level)> = None;
            let mut best_ppm = u32::MAX;
            for venue in 0..MAX_VENUES {
                if let Some(book) = slab.get_book(market_id, outcome as OutcomeId, venue as VenueId)
                {
                    if let Some(level) = book.best() {
                        if level.size > 0 {
                            let eff = config.venue_fees[venue].effective(level.price).ppm();
                            if eff < best_ppm {
                                best_ppm = eff;
                                best = Some((venue, level));
                            }
                        }
                    }
                }
            }
            let (venue, level) = best?;
            let discount = DepthDiscount {
                survival_bps: config.venue_survival_bps[venue],
            };
            let mut levels = [EMPTY_LEVEL; MAX_LEVELS_PER_LEG];
            levels[0] = Level {
                price: level.price,
                size: discount.discounted(level.size),
            };
            out.push(BookLeg {
                venue: venue as VenueId,
                outcome: outcome as OutcomeId,
                fee: config.venue_fees[venue],
                increment: config.venue_increments[venue],
                levels,
                n_levels: 1,
            });
        }
        Some((out, config.budget))
    }

    #[test]
    fn shopping_never_loses_vs_single_best() {
        // Deterministic pseudo-random slab states.
        let mut seed = 0x1234_5678_9abc_def0u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for case in 0..2000 {
            let mut slab = EngineSlab::new(4);
            let mut config = MarketConfig {
                active: true,
                outcome_count: 2 + (next() % 3) as u8, // 2..=4 outcomes
                budget: (next() % 400_000 + 10_000) as i64,
                ..Default::default()
            };
            for v in 0..MAX_VENUES {
                match next() % 4 {
                    0 => config.venue_fees[v] = Fee::None,
                    1 => config.venue_fees[v] = Fee::StakeFeeBps((next() % 500) as u32),
                    2 => config.venue_fees[v] = Fee::CommissionBps((next() % 300) as u32),
                    _ => config.venue_fees[v] = Fee::MakerRebateBps((next() % 100) as u32),
                }
                config.venue_increments[v] = [1, 100][(next() % 2) as usize];
                config.venue_survival_bps[v] = 2500 + (next() % 7500) as u32;
            }
            slab.register_market(0, config).unwrap();

            let mut populated_outcomes = std::collections::HashSet::new();
            for _ in 0..12 {
                let o = (next() % config.outcome_count as u64) as u32;
                let v = (next() % MAX_VENUES as u64) as u16;
                let cents = 20 + (next() % 75) as u32;
                let size = (next() % 90_000 + 40) as i64;
                let levels: Vec<Level> = (0..(next() % 3 + 1))
                    .map(|i| Level {
                        price: Prob::from_cents((cents + i as u32).min(99)).unwrap(),
                        size: size / (i as i64 + 1),
                    })
                    .collect();
                slab.get_book_mut(0, o, v)
                    .unwrap()
                    .apply_snapshot(&levels, 1);
                populated_outcomes.insert(o);
            }

            let shopped = Aggregator::evaluate_market(&slab, 0).unwrap();
            let single = single_best_plan(&slab, 0)
                .and_then(|(legs, budget)| detect_book(&legs, budget).unwrap());

            let shopped_profit = shopped.as_ref().map(|(s, _, _)| s.worst_case_profit);
            let single_profit = single.as_ref().map(|s| s.worst_case_profit);

            // Shopped coverage ⊇ single-best coverage: if single-best found
            // an arb, shopping must too, and never with less profit.
            if let Some(sp) = single_profit {
                assert!(
                    shopped_profit.is_some(),
                    "case {case}: shopping dropped a single-best arbitrage"
                );
                assert!(
                    shopped_profit.unwrap() >= sp,
                    "case {case}: shopping reported {} < single-best {sp}",
                    shopped_profit.unwrap()
                );
            } else if case == 0 {
                eprintln!("note: no baseline arbs generated");
            }
        }
    }
}
