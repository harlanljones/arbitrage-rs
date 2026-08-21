//! Fast market-level quote aggregator and arbitrage detector invocation.
//!
//! Aggregates top-of-book quotes across venues for all outcomes of a given market,
//! applies venue-specific fees and increments, constructs the fixed-size leg array,
//! and invokes [`arbkit_core::detect`].

use crate::error::Result;
use crate::slab::{EngineSlab, MAX_VENUES};
use arbkit_core::book::{MarketId, OutcomeId, VenueId};
use arbkit_core::price::Prob;
use arbkit_core::{detect, Leg, Signal, MAX_LEGS};

/// Market aggregator for zero-allocation quote evaluation.
pub struct Aggregator;

impl Aggregator {
    /// Evaluates all outcomes for `market_id` across all active venues and runs detection.
    ///
    /// Returns `Ok(Some(Signal))` if a tradeable arbitrage exists, `Ok(None)` if no edge
    /// or incomplete quotes exist, or `Err` on core domain errors.
    #[inline]
    pub fn evaluate_market(slab: &EngineSlab, market_id: MarketId) -> Result<Option<Signal>> {
        let config = match slab.get_config(market_id) {
            Some(c) if c.active => c,
            _ => return Ok(None),
        };

        let outcome_count = config.outcome_count as usize;
        let mut legs = [Leg {
            venue: 0,
            outcome: 0,
            quoted: Prob::CERTAIN,
            fee: arbkit_core::Fee::None,
            capacity: 0,
            increment: 1,
        }; MAX_LEGS];

        for (outcome, leg_slot) in legs.iter_mut().enumerate().take(outcome_count) {
            let mut best_effective_ppm = u32::MAX;
            let mut best_leg: Option<Leg> = None;

            for venue in 0..MAX_VENUES {
                let venue_id = venue as VenueId;
                let outcome_id = outcome as OutcomeId;

                if let Some(book) = slab.get_book(market_id, outcome_id, venue_id) {
                    if let Some(level) = book.best() {
                        if level.size > 0 {
                            let fee = config.venue_fees[venue];
                            let effective = fee.effective(level.price);
                            let eff_ppm = effective.ppm();

                            // Smaller ppm = longer price = higher payout = better quote
                            if eff_ppm < best_effective_ppm {
                                best_effective_ppm = eff_ppm;
                                best_leg = Some(Leg {
                                    venue: venue_id,
                                    outcome: outcome_id,
                                    quoted: level.price,
                                    fee,
                                    capacity: level.size,
                                    increment: config.venue_increments[venue],
                                });
                            }
                        }
                    }
                }
            }

            match best_leg {
                Some(leg) => {
                    *leg_slot = leg;
                }
                None => {
                    // One of the outcomes has no valid quote on any venue.
                    // Cannot hedge all sides, so no arb.
                    return Ok(None);
                }
            }
        }

        let signal = detect(&legs[..outcome_count], config.budget)?;
        Ok(signal)
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

        let result = Aggregator::evaluate_market(&slab, 0).unwrap();
        assert!(result.is_some());
        let signal = result.unwrap();
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
        assert_eq!(result, None);
    }
}
