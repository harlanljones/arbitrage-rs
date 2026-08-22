//! Preallocated engine state and flat order book slab.
//!
//! Hot path lookups are direct integer-indexed O(1) arithmetic operations with
//! zero allocations and no hashmap lookups.

use crate::error::{EngineError, Result};
use arbkit_core::book::{Cents, MarketId, OutcomeBook, OutcomeId, VenueId};
use arbkit_core::{Fee, MAX_LEGS};

/// Default maximum number of markets preallocated in the slab.
pub const DEFAULT_MAX_MARKETS: usize = 1024;

/// Maximum number of outcomes supported per market.
pub const MAX_OUTCOMES: usize = MAX_LEGS;

/// Maximum number of venues supported per outcome.
pub const MAX_VENUES: usize = 8;

/// Configuration and fee parameters for an active market.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarketConfig {
    /// Number of mutually exclusive outcomes for this market (2 to 4).
    pub outcome_count: u8,
    /// Per-venue fee structures applied before arbitrage detection.
    pub venue_fees: [Fee; MAX_VENUES],
    /// Per-venue stake increments in cents (e.g. 1 cent or contract price).
    pub venue_increments: [Cents; MAX_VENUES],
    /// Per-venue share of resting depth expected to survive transit to the
    /// venue, in basis points (`10_000` = untouched). Detection sizes against
    /// the discounted figure so a signal never requests more than the sim's
    /// fill model will actually honor.
    pub venue_survival_bps: [u32; MAX_VENUES],
    /// Minimum nanoseconds between two emitted signals for this market
    /// (`0` = emit on every detection). Duplicate emissions of an unchanged
    /// edge within the window are suppressed at emission time.
    pub signal_cooldown_ns: u64,
    /// Maximum stake budget in cents evaluated during detection.
    pub budget: Cents,
    /// Whether this market is currently active and eligible for detection.
    pub active: bool,
}

impl Default for MarketConfig {
    fn default() -> Self {
        Self {
            outcome_count: 2,
            venue_fees: [Fee::None; MAX_VENUES],
            venue_increments: [1; MAX_VENUES],
            venue_survival_bps: [10_000; MAX_VENUES],
            signal_cooldown_ns: 0,
            budget: 100_000,
            active: false,
        }
    }
}

/// Preallocated slab holding all [`OutcomeBook`]s and market configurations.
#[derive(Debug, Clone)]
pub struct EngineSlab {
    books: Vec<OutcomeBook>,
    configs: Vec<MarketConfig>,
    /// Emission-time clock reading of the last emitted signal, per market.
    ///
    /// Runtime state rather than a `MarketConfig` field: configs are `Copy`
    /// values overwritten wholesale by [`EngineSlab::register_market`], while
    /// the cooldown bookkeeping must survive re-registration untouched.
    ///
    /// A slot of `0` means the market has never emitted, and always admits:
    /// a fresh market must not wait out a window it never opened. Should an
    /// emit clock ever legitimately read exactly `0`, the failure direction
    /// is safe — one extra emission, never a lost edge.
    last_emit_ns: Vec<u64>,
    max_markets: usize,
}

impl EngineSlab {
    /// Creates a new slab preallocating storage for up to `max_markets`.
    pub fn new(max_markets: usize) -> Self {
        let total_books = max_markets * MAX_OUTCOMES * MAX_VENUES;
        let mut books = Vec::with_capacity(total_books);
        for _ in 0..total_books {
            books.push(OutcomeBook::new());
        }

        let mut configs = Vec::with_capacity(max_markets);
        for _ in 0..max_markets {
            configs.push(MarketConfig::default());
        }

        Self {
            books,
            configs,
            last_emit_ns: vec![0; max_markets],
            max_markets,
        }
    }

    /// Computes the flat slab index for a `(market_id, outcome_id, venue_id)` tuple.
    #[inline]
    fn flat_index(
        &self,
        market_id: MarketId,
        outcome_id: OutcomeId,
        venue_id: VenueId,
    ) -> Option<usize> {
        let m = market_id as usize;
        let o = outcome_id as usize;
        let v = venue_id as usize;

        if m >= self.max_markets || o >= MAX_OUTCOMES || v >= MAX_VENUES {
            return None;
        }

        Some((m * MAX_OUTCOMES + o) * MAX_VENUES + v)
    }

    /// Registers a market's configuration in the slab.
    pub fn register_market(&mut self, market_id: MarketId, config: MarketConfig) -> Result<()> {
        let m = market_id as usize;
        if m >= self.max_markets {
            return Err(EngineError::MarketOutOfRange {
                market_id,
                capacity: self.max_markets,
            });
        }
        if !(2..=4).contains(&config.outcome_count) {
            return Err(EngineError::Core(
                arbkit_core::ArbError::LegCountOutOfRange(config.outcome_count as usize),
            ));
        }

        self.configs[m] = config;
        Ok(())
    }

    /// Returns a reference to the [`OutcomeBook`] for the given tuple, or `None` if out of bounds.
    #[inline]
    pub fn get_book(
        &self,
        market_id: MarketId,
        outcome_id: OutcomeId,
        venue_id: VenueId,
    ) -> Option<&OutcomeBook> {
        let idx = self.flat_index(market_id, outcome_id, venue_id)?;
        Some(&self.books[idx])
    }

    /// Returns a mutable reference to the [`OutcomeBook`] for the given tuple.
    #[inline]
    pub fn get_book_mut(
        &mut self,
        market_id: MarketId,
        outcome_id: OutcomeId,
        venue_id: VenueId,
    ) -> Option<&mut OutcomeBook> {
        let idx = self.flat_index(market_id, outcome_id, venue_id)?;
        Some(&mut self.books[idx])
    }

    /// Returns a reference to the [`MarketConfig`] for `market_id`.
    #[inline]
    pub fn get_config(&self, market_id: MarketId) -> Option<&MarketConfig> {
        let m = market_id as usize;
        if m < self.max_markets {
            Some(&self.configs[m])
        } else {
            None
        }
    }

    /// Returns a mutable reference to the [`MarketConfig`] for `market_id`.
    #[inline]
    pub fn get_config_mut(&mut self, market_id: MarketId) -> Option<&mut MarketConfig> {
        let m = market_id as usize;
        if m < self.max_markets {
            Some(&mut self.configs[m])
        } else {
            None
        }
    }

    /// Returns the maximum number of markets supported in this slab.
    #[inline]
    pub fn max_markets(&self) -> usize {
        self.max_markets
    }

    /// Returns `true` when a signal for `market_id` may be emitted at
    /// `now_ns` under the market's cooldown, i.e. when no signal was emitted
    /// within the configured window. A disabled cooldown (`0`) always admits.
    ///
    /// This only *reads* the gate; [`EngineSlab::note_emit`] records the
    /// emission afterwards so a suppressed attempt cannot extend its own
    /// suppression window.
    #[inline]
    pub fn emit_admitted(&self, market_id: MarketId, now_ns: u64) -> bool {
        let m = market_id as usize;
        if m >= self.max_markets {
            return false;
        }
        let cooldown = self.configs[m].signal_cooldown_ns;
        if cooldown == 0 {
            return true;
        }
        let last = self.last_emit_ns[m];
        !(last != 0 && now_ns.saturating_sub(last) < cooldown)
    }

    /// Records that a signal for `market_id` was emitted at `now_ns`,
    /// starting (or restarting) its cooldown window. A `0` clock reading
    /// leaves the "never emitted" marker in place, which admits.
    #[inline]
    pub fn note_emit(&mut self, market_id: MarketId, now_ns: u64) {
        let m = market_id as usize;
        if m < self.max_markets && now_ns != 0 {
            self.last_emit_ns[m] = now_ns;
        }
    }

    /// Resets all books to empty/stale state and deactivates configs.
    pub fn reset(&mut self) {
        for book in &mut self.books {
            *book = OutcomeBook::new();
        }
        for config in &mut self.configs {
            *config = MarketConfig::default();
        }
        self.last_emit_ns.fill(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arbkit_core::book::Level;
    use arbkit_core::Prob;

    #[test]
    fn test_slab_indexing_and_updates() {
        let mut slab = EngineSlab::new(10);
        let config = MarketConfig {
            active: true,
            outcome_count: 2,
            budget: 50_000,
            ..Default::default()
        };

        assert!(slab.register_market(0, config).is_ok());
        assert!(slab.get_config(0).unwrap().active);

        let book = slab.get_book_mut(0, 1, 2).unwrap();
        assert!(book.is_stale());

        book.apply_snapshot(
            &[Level {
                price: Prob::from_cents(50).unwrap(),
                size: 1_000,
            }],
            1,
        );

        let read_book = slab.get_book(0, 1, 2).unwrap();
        assert!(!read_book.is_stale());
        assert_eq!(read_book.best().unwrap().price.ppm(), 500_000);
    }

    #[test]
    fn test_slab_out_of_bounds() {
        let mut slab = EngineSlab::new(4);
        assert!(slab.get_book(4, 0, 0).is_none());
        assert!(slab.get_book(0, 4, 0).is_none());
        assert!(slab.get_book(0, 0, 8).is_none());

        let config = MarketConfig::default();
        assert!(slab.register_market(4, config).is_err());
    }

    #[test]
    fn test_emit_cooldown_gate_and_reset() {
        let mut slab = EngineSlab::new(4);
        let config = MarketConfig {
            active: true,
            signal_cooldown_ns: 500,
            ..Default::default()
        };
        slab.register_market(0, config).unwrap();

        // Disabled cooldown (default config) always admits.
        assert!(slab.emit_admitted(1, 10_000));

        // First emission is always admitted; it opens the window.
        assert!(slab.emit_admitted(0, 1_000));
        slab.note_emit(0, 1_000);
        assert!(!slab.emit_admitted(0, 1_200));
        assert!(!slab.emit_admitted(0, 1_400));
        // Exactly at the window edge the market admits again.
        assert!(slab.emit_admitted(0, 1_500));

        // Re-registration replaces the config but not the runtime gate:
        // the window opened by the first emission still binds.
        assert!(!slab.emit_admitted(0, 1_100));
        slab.register_market(
            0,
            MarketConfig {
                signal_cooldown_ns: 200,
                ..config
            },
        )
        .unwrap();
        assert!(!slab.emit_admitted(0, 1_150));
        assert!(slab.emit_admitted(0, 1_200));

        // Reset clears emission bookkeeping along with everything else.
        slab.reset();
        assert!(slab.emit_admitted(0, 1_100));
    }
}
