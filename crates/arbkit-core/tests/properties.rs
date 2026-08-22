#![allow(deprecated)]

//! Properties that must hold for every price and every set of legs.
//!
//! The unit tests pin down specific numbers against published sources. These
//! pin down the invariants the rest of the system leans on — above all, that a
//! [`Signal`] is genuinely risk-free as reported, for *any* input that
//! produces one. That claim is the whole product, and an example-based test
//! can only ever check it in the places one thought to look.

use arbkit_core::arb::{detect, detect_book, BookLeg, Leg, MAX_CHUNKS, MAX_LEVELS_PER_LEG};
use arbkit_core::book::{Cents, Level, OutcomeId};
use arbkit_core::fee::Fee;
use arbkit_core::price::{Odds, Prob, ODDS_ONE, PPM};
use proptest::prelude::*;

/// Any valid price.
fn any_prob() -> impl Strategy<Value = Prob> {
    (1u32..=PPM).prop_map(|ppm| Prob::from_ppm(ppm).unwrap())
}

/// Any valid fee, including none.
fn any_fee() -> impl Strategy<Value = Fee> {
    prop_oneof![
        Just(Fee::None),
        (0u32..=1_000).prop_map(Fee::CommissionBps),
        (0u32..=1_000).prop_map(Fee::StakeFeeBps),
    ]
}

/// A leg with plausible depth and granularity.
fn any_leg() -> impl Strategy<Value = Leg> {
    (any_prob(), any_fee(), 1i64..10_000_000i64, 1i64..10_000i64).prop_map(
        |(quoted, fee, capacity, increment)| Leg {
            venue: 0,
            outcome: 0,
            quoted,
            fee,
            capacity,
            increment,
        },
    )
}

/// A one-level book leg carrying the same quote as `leg`.
fn as_book_leg(leg: &Leg, outcome: OutcomeId) -> BookLeg {
    let mut levels = [Level {
        price: Prob::CERTAIN,
        size: 0,
    }; MAX_LEVELS_PER_LEG];
    levels[0] = Level {
        price: leg.quoted,
        size: leg.capacity,
    };
    BookLeg {
        venue: leg.venue,
        outcome,
        fee: leg.fee,
        increment: leg.increment,
        levels,
        n_levels: 1,
    }
}

/// A book leg several levels deep, with `outcomes` distinct outcomes to land on.
///
/// Deliberately includes the shapes that used to be special cases: zero levels
/// (a stale or empty book), several legs sharing an outcome, and levels whose
/// resting size is smaller than one tradeable increment.
fn any_book_leg(
    outcomes: u32,
    increments: impl Strategy<Value = Cents>,
) -> impl Strategy<Value = BookLeg> {
    (
        0..outcomes,
        any_fee(),
        increments,
        prop::collection::vec((any_prob(), 0i64..1_000_000i64), 0..=MAX_LEVELS_PER_LEG),
    )
        .prop_map(|(outcome, fee, increment, levels)| {
            let mut filled = [Level {
                price: Prob::CERTAIN,
                size: 0,
            }; MAX_LEVELS_PER_LEG];
            for (slot, (price, size)) in filled.iter_mut().zip(&levels) {
                *slot = Level {
                    price: *price,
                    size: *size,
                };
            }
            BookLeg {
                venue: 0,
                outcome,
                fee,
                increment,
                levels: filled,
                n_levels: levels.len() as u8,
            }
        })
}

/// The guaranteed profit of the closed-form equal-payoff plan, in cents.
///
/// This is the sizing `detect` used before it could see past the top of book,
/// reproduced here as an independent reference: stake each leg a share of the
/// total proportional to its fee-adjusted price, cap the total at whatever the
/// thinnest leg supports, floor every stake to the venue's increment, floor
/// every payout, and report the worst outcome's payout less the total staked.
///
/// It is deliberately a transcription rather than a call into the crate. The
/// point of comparing against it is that the new search must never do worse,
/// and a reference that shares code with the thing under test cannot show that.
fn closed_form_profit(legs: &[Leg], budget: Cents) -> Option<i128> {
    if budget <= 0 {
        return None;
    }
    let ppm = i128::from(PPM);
    let mut effective = [0i128; MAX_CHUNKS];
    let mut overround = 0i128;
    for (slot, leg) in legs.iter().enumerate() {
        if leg.increment <= 0 || leg.capacity < leg.increment {
            return None;
        }
        effective[slot] = i128::from(leg.fee.effective(leg.quoted).ppm());
        overround += effective[slot];
    }
    if overround >= ppm {
        return None;
    }

    let mut total_budget = budget as i128;
    for (slot, leg) in legs.iter().enumerate() {
        total_budget = total_budget.min((leg.capacity as i128) * overround / effective[slot]);
    }
    if total_budget <= 0 {
        return None;
    }

    let mut total_stake = 0i128;
    let mut worst_payout = i128::MAX;
    for (slot, leg) in legs.iter().enumerate() {
        let increment = leg.increment as i128;
        let stake = (total_budget * effective[slot] / overround / increment) * increment;
        if stake <= 0 {
            return None;
        }
        total_stake += stake;
        worst_payout = worst_payout.min(stake * ppm / effective[slot]);
    }

    let profit = worst_payout - total_stake;
    (profit > 0).then_some(profit)
}

proptest! {
    /// A price survives the trip through its own reciprocal.
    ///
    /// Both directions are lossy by a rounding step, so the bound is one ppm
    /// rather than exact equality — but it must be one ppm, not "close".
    #[test]
    fn prob_round_trips_through_odds(prob in any_prob()) {
        let returned = prob.to_odds().to_prob();
        let drift = prob.ppm().abs_diff(returned.ppm());
        prop_assert!(drift <= 1, "{} ppm -> {} ppm", prob.ppm(), returned.ppm());
    }

    /// Decimal odds survive the trip through implied probability.
    ///
    /// Odds are the lossy representation, and deliberately so. Probability is
    /// quantized to a part per million, which at 1.01 is a rounding hair and
    /// at 500.0 is a swing of several whole points of odds — so the tolerance
    /// has to be stated in the units the quantization happens in. What must
    /// hold exactly is the thing determinism depends on: the canonical form
    /// is a fixed point, so replaying a tape cannot drift.
    #[test]
    fn odds_round_trip_through_prob(micro in ODDS_ONE..=(ODDS_ONE * 10_000)) {
        let odds = Odds::from_micro(micro).unwrap();
        let prob = odds.to_prob();
        let returned = prob.to_odds();

        prop_assert_eq!(returned.to_prob(), prob, "canonical form must be stable");

        // Within one ppm-step of probability, which as a share of the odds is
        // 1 / prob_ppm.
        let drift = micro.abs_diff(returned.micro());
        prop_assert!(
            drift * u64::from(prob.ppm()) <= micro * 2,
            "{micro} -> {} at {} ppm",
            returned.micro(),
            prob.ppm(),
        );
    }

    /// American odds survive the trip through implied probability.
    ///
    /// Restricted to the range a venue actually quotes; the notation gets
    /// coarse enough past that to lose a ppm-scale price on the way back.
    #[test]
    fn american_odds_round_trip(american in -5_000i32..=5_000i32) {
        prop_assume!(american.abs() >= 100);
        let prob = Prob::from_american(american).unwrap();
        // Even money (-100 and +100) canonicalizes to +100 by convention.
        let expected = if american == -100 { 100 } else { american };
        prop_assert_eq!(prob.to_american(), Some(expected));
    }

    /// Ordering is preserved by the reciprocal, in reverse.
    #[test]
    fn a_shorter_price_is_always_shorter_odds(a in any_prob(), b in any_prob()) {
        prop_assume!(a != b);
        prop_assert_eq!(a < b, a.to_odds() > b.to_odds());
    }

    /// A fee can never make a price better.
    ///
    /// If this ever fails, some fee has become a rebate and the detector will
    /// start finding edges that the venue is not offering.
    #[test]
    fn fees_only_ever_make_a_price_worse(prob in any_prob(), fee in any_fee()) {
        prop_assert!(fee.effective(prob) >= prob);
    }

    /// A signal is risk-free, as reported, for every input that produces one.
    ///
    /// This is the property the whole system rests on: whichever outcome
    /// settles, the return covers the total staked with at least the profit
    /// the signal claims. Everything else is an optimization.
    #[test]
    fn every_signal_is_actually_risk_free(
        legs in prop::collection::vec(any_leg(), 2..=4),
        budget in 1i64..1_000_000_000i64,
    ) {
        let Some(signal) = detect(&legs, budget).unwrap() else {
            return Ok(());
        };

        prop_assert!(signal.worst_case_profit > 0);
        prop_assert!(signal.total_stake > 0);
        prop_assert!(signal.total_stake <= budget);
        prop_assert_eq!(signal.allocations().len(), legs.len());

        for allocation in signal.allocations() {
            let leg = &legs[allocation.leg];

            // Tradeable: within the depth on offer, and a whole number of
            // whatever unit the venue deals in.
            prop_assert!(allocation.stake > 0);
            prop_assert!(allocation.stake <= leg.capacity);
            prop_assert_eq!(allocation.stake % leg.increment, 0);

            // Risk-free: this leg alone repays everything staked, plus the
            // profit the signal promised.
            let net: Cents = allocation.payout - signal.total_stake;
            prop_assert!(
                net >= signal.worst_case_profit,
                "leg {} nets {} but the signal promised {}",
                allocation.leg, net, signal.worst_case_profit,
            );
        }

        let staked: Cents = signal.allocations().iter().map(|a| a.stake).sum();
        prop_assert_eq!(staked, signal.total_stake);
    }

    /// No signal is ever produced at or above parity.
    ///
    /// The mirror of the property above: not just "what it emits is sound",
    /// but "it emits nothing when the prices do not support it".
    #[test]
    fn parity_or_worse_never_produces_a_signal(
        legs in prop::collection::vec(any_leg(), 2..=4),
        budget in 1i64..1_000_000_000i64,
    ) {
        let overround: u64 = legs
            .iter()
            .map(|leg| u64::from(leg.fee.effective(leg.quoted).ppm()))
            .sum();
        prop_assume!(overround >= u64::from(PPM));
        prop_assert_eq!(detect(&legs, budget).unwrap(), None);
    }

    /// Detection never panics, whatever the input.
    ///
    /// The hot path builds in release with `panic = "abort"`, so a panic here
    /// is not an exception to catch — it is the process going away mid-trade.
    #[test]
    fn detection_is_total(
        legs in prop::collection::vec(any_leg(), 0..=6),
        budget in -1_000i64..i64::MAX,
    ) {
        let _ = detect(&legs, budget);
    }

    /// Book detection never panics either, whatever the book looks like.
    ///
    /// The generator reaches the cases that used to be handled by an early
    /// return and are now handled by the search: legs with no levels at all,
    /// several legs quoting the same outcome, levels whose resting size is
    /// under one increment, a zero or negative increment, and leg counts on
    /// both sides of the accepted range.
    #[test]
    fn book_detection_is_total(
        legs in prop::collection::vec(any_book_leg(3, -2i64..10_000i64), 0..=(MAX_CHUNKS + 2)),
        budget in -1_000i64..i64::MAX,
    ) {
        let _ = detect_book(&legs, budget);
    }

    /// The book search never returns less guaranteed profit than the sizing it
    /// replaced.
    ///
    /// The closed-form equal-payoff plan is a member of the new feasible set
    /// by construction, so anything the search reports has to be at least as
    /// good — and if the closed form found a trade at all, the search must
    /// find one too. This is the property that makes the rewrite a strict
    /// improvement rather than a different set of trade-offs.
    #[test]
    fn the_book_search_never_loses_to_the_closed_form(
        legs in prop::collection::vec(any_leg(), 2..=4),
        budget in 1i64..1_000_000_000i64,
    ) {
        // One outcome per leg, which is what the closed form assumes.
        let books: Vec<BookLeg> = legs
            .iter()
            .enumerate()
            .map(|(i, leg)| as_book_leg(leg, i as OutcomeId))
            .collect();

        let reference = closed_form_profit(&legs, budget);
        let searched = detect_book(&books, budget).unwrap();

        match reference {
            None => {} // The search is allowed to find edges the closed form missed.
            Some(baseline) => {
                let signal = searched.expect("the closed-form plan is always feasible");
                prop_assert!(
                    i128::from(signal.worst_case_profit) >= baseline,
                    "search cleared {} against the closed form's {baseline}",
                    signal.worst_case_profit,
                );
            }
        }
    }

    /// A book signal never claims more than its own allocations deliver.
    ///
    /// Recomputed from scratch — group the allocations back onto their
    /// outcomes, sum the payouts, subtract the total staked — every outcome
    /// must cover the profit the signal reports. The signal is also checked
    /// against the book it came from: nothing staked past the resting size of
    /// a leg, nothing off-increment, nothing above budget, and no payout
    /// larger than the leg's best price could pay for that stake.
    #[test]
    fn a_book_signal_never_overstates_what_it_delivers(
        legs in prop::collection::vec(any_book_leg(3, 1i64..1_000i64), 2..=6),
        budget in 1i64..1_000_000_000i64,
    ) {
        let Ok(Some(signal)) = detect_book(&legs, budget) else {
            return Ok(());
        };

        prop_assert!(signal.worst_case_profit > 0);
        prop_assert!(signal.total_stake > 0);
        prop_assert!(signal.total_stake <= budget);

        let staked: Cents = signal.allocations().iter().map(|a| a.stake).sum();
        prop_assert_eq!(staked, signal.total_stake);

        // Per-leg: inside the depth, on the increment, and paying no more than
        // the leg's best price would.
        let mut per_leg = [0 as Cents; MAX_CHUNKS];
        for allocation in signal.allocations() {
            let leg = &legs[allocation.leg];
            prop_assert!(allocation.stake > 0);
            prop_assert_eq!(allocation.stake % leg.increment, 0);
            per_leg[allocation.leg] += allocation.stake;

            let best = leg.levels[..leg.n_levels as usize]
                .iter()
                .map(|level| i128::from(leg.fee.effective(level.price).ppm()))
                .min()
                .expect("an allocation implies a usable level");
            let ceiling = i128::from(allocation.stake) * i128::from(PPM) / best;
            prop_assert!(
                i128::from(allocation.payout) <= ceiling,
                "leg {} claims {} on {} staked, best price pays {ceiling}",
                allocation.leg, allocation.payout, allocation.stake,
            );
        }
        for (i, leg) in legs.iter().enumerate() {
            let resting: Cents = leg.levels[..leg.n_levels as usize]
                .iter()
                .map(|level| level.size)
                .sum();
            prop_assert!(per_leg[i] <= resting, "leg {i} staked {} into {resting}", per_leg[i]);
        }

        // Per-outcome: whichever side settles, the return covers everything
        // staked plus the profit that was promised.
        let mut outcomes = [(0 as OutcomeId, 0 as Cents); MAX_CHUNKS];
        let mut n = 0usize;
        for allocation in signal.allocations() {
            let outcome = legs[allocation.leg].outcome;
            let slot = match outcomes[..n].iter().position(|(o, _)| *o == outcome) {
                Some(slot) => slot,
                None => {
                    outcomes[n] = (outcome, 0);
                    n += 1;
                    n - 1
                }
            };
            outcomes[slot].1 += allocation.payout;
        }
        // Every outcome that has a usable quote must be backed, or the hedge
        // is partial and the "guaranteed" profit is a directional bet.
        let quoted: usize = {
            let mut seen = [0 as OutcomeId; MAX_CHUNKS];
            let mut count = 0usize;
            for leg in &legs {
                if !seen[..count].contains(&leg.outcome) {
                    seen[count] = leg.outcome;
                    count += 1;
                }
            }
            count
        };
        prop_assert_eq!(n, quoted, "only {} of {} outcomes were backed", n, quoted);

        for (outcome, payout) in &outcomes[..n] {
            prop_assert!(
                payout - signal.total_stake >= signal.worst_case_profit,
                "outcome {outcome} nets {} but the signal promised {}",
                payout - signal.total_stake,
                signal.worst_case_profit,
            );
        }
    }
}
