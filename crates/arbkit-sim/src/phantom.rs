//! Phantom arbitrage detection, classification, and statistical tracking.
//!
//! A "phantom arb" is an arbitrage opportunity detected in feed data that could
//! not be monetized in practice. The phantom rate is the single most important
//! metric for evaluating an arbitrage bot's true viability: an algorithm that
//! finds 10,000 opportunities a day with a 99% phantom rate will bleed money on
//! broken legs and exchange fees.

use arbkit_core::VenueId;

/// One basis point denominator (100% = 10,000 bps).
const BPS: u64 = 10_000;

/// Specific root cause why an arbitrage opportunity became a phantom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhantomReason {
    /// Quoted price moved or disappeared before the simulated order arrived.
    PriceMoved {
        /// The venue on which the price moved.
        venue: VenueId,
    },
    /// Resting depth was exhausted or front-run in the queue.
    DepthExhausted {
        /// The venue with insufficient depth.
        venue: VenueId,
    },
    /// The book became stale or disconnected during the transit window.
    BookStale {
        /// The venue with the stale book.
        venue: VenueId,
    },
    /// One leg filled but an opposing leg failed, creating an open unhedged position.
    BrokenLeg {
        /// Venue where the leg filled.
        filled_venue: VenueId,
        /// Venue where the leg failed.
        failed_venue: VenueId,
    },
    /// Legs were partially filled in an unbalanced ratio that destroyed the risk-free property.
    AsymmetricFill,
    /// Slippage, rounding, or fee deductions turned the net worst-case return negative.
    UnprofitableAfterCosts,
}

/// Overall execution classification of an arbitrage opportunity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArbExecutionClassification {
    /// Fully filled on all legs as detected.
    CleanFill,
    /// Proportionally filled across all legs, preserving risk-free hedge.
    ProportionalPartialFill,
    /// The opportunity was a phantom and could not be executed cleanly.
    Phantom(PhantomReason),
}

impl ArbExecutionClassification {
    /// Whether this execution was a phantom.
    #[inline]
    pub const fn is_phantom(&self) -> bool {
        matches!(self, ArbExecutionClassification::Phantom(_))
    }

    /// Whether this execution resulted in an unhedged broken leg.
    #[inline]
    pub const fn is_broken_leg(&self) -> bool {
        matches!(
            self,
            ArbExecutionClassification::Phantom(PhantomReason::BrokenLeg { .. })
        )
    }

    /// Whether this execution was cleanly and profitably executed.
    #[inline]
    pub const fn is_clean(&self) -> bool {
        matches!(self, ArbExecutionClassification::CleanFill)
    }
}

/// Aggregated statistics measuring phantom arbitrage frequency and causes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PhantomStats {
    /// Total number of detected arbitrage signals evaluated.
    pub total_detected: u64,
    /// Number of signals completely filled as planned.
    pub clean_fills: u64,
    /// Number of signals partially filled with preserved hedge.
    pub proportional_fills: u64,
    /// Total number of phantom arbitrage occurrences.
    pub total_phantoms: u64,
    /// Phantoms caused by price movement / quote decay.
    pub phantoms_price_moved: u64,
    /// Phantoms caused by resting depth exhaustion.
    pub phantoms_depth_exhausted: u64,
    /// Phantoms caused by stale / dropped books.
    pub phantoms_book_stale: u64,
    /// Phantoms caused by broken legs (unhedged directional risk).
    pub phantoms_broken_leg: u64,
    /// Phantoms caused by asymmetric / unbalanced partial fills.
    pub phantoms_asymmetric_fill: u64,
    /// Phantoms where fees or slippage ate all profit.
    pub phantoms_unprofitable: u64,
}

impl PhantomStats {
    /// Record a classified execution event.
    pub fn record(&mut self, classification: ArbExecutionClassification) {
        self.total_detected += 1;
        match classification {
            ArbExecutionClassification::CleanFill => {
                self.clean_fills += 1;
            }
            ArbExecutionClassification::ProportionalPartialFill => {
                self.proportional_fills += 1;
            }
            ArbExecutionClassification::Phantom(reason) => {
                self.total_phantoms += 1;
                match reason {
                    PhantomReason::PriceMoved { .. } => self.phantoms_price_moved += 1,
                    PhantomReason::DepthExhausted { .. } => self.phantoms_depth_exhausted += 1,
                    PhantomReason::BookStale { .. } => self.phantoms_book_stale += 1,
                    PhantomReason::BrokenLeg { .. } => self.phantoms_broken_leg += 1,
                    PhantomReason::AsymmetricFill => self.phantoms_asymmetric_fill += 1,
                    PhantomReason::UnprofitableAfterCosts => self.phantoms_unprofitable += 1,
                }
            }
        }
    }

    /// The phantom rate in basis points (`total_phantoms * 10000 / total_detected`).
    #[inline]
    pub fn phantom_rate_bps(&self) -> u32 {
        if self.total_detected == 0 {
            return 0;
        }
        ((self.total_phantoms as u128 * BPS as u128) / (self.total_detected as u128)) as u32
    }

    /// The phantom rate as a floating-point fraction in `0.0..=1.0` (for reporting).
    #[inline]
    pub fn phantom_rate_f64(&self) -> f64 {
        if self.total_detected == 0 {
            return 0.0;
        }
        self.total_phantoms as f64 / self.total_detected as f64
    }

    /// The broken leg rate in basis points (`phantoms_broken_leg * 10000 / total_detected`).
    #[inline]
    pub fn broken_leg_rate_bps(&self) -> u32 {
        if self.total_detected == 0 {
            return 0;
        }
        ((self.phantoms_broken_leg as u128 * BPS as u128) / (self.total_detected as u128)) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phantom_rate_accounting() {
        let mut stats = PhantomStats::default();
        stats.record(ArbExecutionClassification::CleanFill);
        stats.record(ArbExecutionClassification::CleanFill);
        stats.record(ArbExecutionClassification::Phantom(
            PhantomReason::PriceMoved { venue: 1 },
        ));
        stats.record(ArbExecutionClassification::Phantom(
            PhantomReason::BrokenLeg {
                filled_venue: 0,
                failed_venue: 1,
            },
        ));

        assert_eq!(stats.total_detected, 4);
        assert_eq!(stats.clean_fills, 2);
        assert_eq!(stats.total_phantoms, 2);
        assert_eq!(stats.phantoms_broken_leg, 1);
        assert_eq!(stats.phantom_rate_bps(), 5_000); // 50.00%
        assert_eq!(stats.broken_leg_rate_bps(), 2_500); // 25.00%
    }
}
