//! Per-venue order book state.
//!
//! One [`OutcomeBook`] per (venue, outcome) pair. Capacity is fixed at
//! construction and never grows: the hot path must not allocate, and a book
//! that reallocates mid-update is both a latency spike and a cache miss on the
//! next read.
//!
//! # Staleness is a first-class state
//!
//! Exchange feeds are a snapshot followed by sequence-numbered deltas. When a
//! sequence number is skipped, the local book is *wrong* and there is no way
//! to interpolate the missing change — the only correct move is to stop
//! trusting it, reconnect, and wait for a fresh snapshot. A book that has lost
//! the sequence reports [`OutcomeBook::is_stale`] and yields no best price, so
//! a gap degrades into silence rather than into confidently wrong signals.

use crate::price::Prob;

/// A venue, interned to an integer at the feed boundary.
pub type VenueId = u16;

/// One side of one market, interned to an integer by the matcher.
pub type OutcomeId = u32;

/// A market — a set of mutually exclusive outcomes — interned by the matcher.
pub type MarketId = u32;

/// Money in whole cents. Signed, because profit can be negative.
pub type Cents = i64;

/// The most price levels a single outcome retains.
///
/// Arbitrage is decided at the top of book and sized against the first few
/// levels behind it. Depth past that is someone else's problem, and every
/// level costs cache.
pub const MAX_LEVELS: usize = 8;

/// A resting price and the stake it can absorb.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Level {
    /// The implied probability on offer.
    pub price: Prob,
    /// How much stake this level can take, in cents.
    pub size: Cents,
}

/// The book for one outcome at one venue.
///
/// Levels are held best-first, where "best" means the *longest* price — the
/// smallest [`Prob`] — since that is the one paying the most.
#[derive(Debug, Clone)]
pub struct OutcomeBook {
    levels: [Level; MAX_LEVELS],
    len: u8,
    stale: bool,
    last_seq: u64,
}

impl OutcomeBook {
    /// An empty book, stale until its first snapshot arrives.
    ///
    /// Starting stale is deliberate: a freshly constructed book has never been
    /// told anything, and "I have no data" and "the price is empty" must not
    /// look the same to the detector.
    pub fn new() -> OutcomeBook {
        OutcomeBook {
            levels: [Level {
                price: Prob::CERTAIN,
                size: 0,
            }; MAX_LEVELS],
            len: 0,
            stale: true,
            last_seq: 0,
        }
    }

    /// Replace the whole book from a snapshot, clearing staleness.
    ///
    /// Levels beyond [`MAX_LEVELS`] are dropped. Input is sorted best-first.
    pub fn apply_snapshot(&mut self, levels: &[Level], seq: u64) {
        let take = levels.len().min(MAX_LEVELS);
        self.levels[..take].copy_from_slice(&levels[..take]);
        self.levels[..take].sort_unstable_by_key(|level| level.price);
        self.len = take as u8;
        self.last_seq = seq;
        self.stale = false;
    }

    /// Accept the next sequenced update, or go stale on a gap.
    ///
    /// Returns `false` when the sequence broke, which the caller should treat
    /// as "reconnect and resubscribe", not as "retry this message".
    #[inline]
    pub fn accept_seq(&mut self, seq: u64) -> bool {
        if self.stale || seq != self.last_seq + 1 {
            self.stale = true;
            self.len = 0;
            return false;
        }
        self.last_seq = seq;
        true
    }

    /// Whether this book has lost the sequence and must not be traded on.
    #[inline]
    pub const fn is_stale(&self) -> bool {
        self.stale
    }

    /// Mark the book untrustworthy — on disconnect, or on a venue-side halt.
    #[inline]
    pub fn mark_stale(&mut self) {
        self.stale = true;
        self.len = 0;
    }

    /// The best price and its size, or `None` if empty or stale.
    #[inline]
    pub fn best(&self) -> Option<Level> {
        if self.stale || self.len == 0 {
            None
        } else {
            Some(self.levels[0])
        }
    }

    /// The retained levels, best-first. Empty while stale.
    #[inline]
    pub fn levels(&self) -> &[Level] {
        if self.stale {
            &[]
        } else {
            &self.levels[..self.len as usize]
        }
    }

    /// Total stake absorbable at or better than `limit`.
    ///
    /// Sizing an arb against the top level alone overstates it whenever the
    /// top level is thin, which on a prediction market is most of the time.
    pub fn depth_to(&self, limit: Prob) -> Cents {
        self.levels()
            .iter()
            .take_while(|level| level.price <= limit)
            .map(|level| level.size)
            .sum()
    }
}

impl Default for OutcomeBook {
    fn default() -> Self {
        OutcomeBook::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level(cents: u32, size: Cents) -> Level {
        Level {
            price: Prob::from_cents(cents).unwrap(),
            size,
        }
    }

    #[test]
    fn a_new_book_is_stale_not_empty() {
        // "I have never been told anything" must not be confused with
        // "the price is empty", or the detector will trade on a book that
        // was never populated.
        let book = OutcomeBook::new();
        assert!(book.is_stale());
        assert_eq!(book.best(), None);
    }

    #[test]
    fn a_snapshot_clears_staleness_and_sorts_best_first() {
        let mut book = OutcomeBook::new();
        book.apply_snapshot(&[level(55, 1_000), level(52, 500)], 7);
        assert!(!book.is_stale());
        // Best means longest price, so the smallest implied probability.
        assert_eq!(book.best().unwrap().price.ppm(), 520_000);
    }

    #[test]
    fn a_sequence_gap_takes_the_book_out_of_service() {
        let mut book = OutcomeBook::new();
        book.apply_snapshot(&[level(52, 500)], 7);
        assert!(book.accept_seq(8));

        // 9 is missing. There is no way to interpolate what it said.
        assert!(!book.accept_seq(10));
        assert!(book.is_stale());
        assert_eq!(book.best(), None);

        // And it stays out of service until a fresh snapshot arrives — a
        // resumed sequence must not silently re-enable a wrong book.
        assert!(!book.accept_seq(11));
        book.apply_snapshot(&[level(52, 500)], 11);
        assert!(book.best().is_some());
    }

    #[test]
    fn depth_accumulates_across_levels_up_to_a_limit() {
        let mut book = OutcomeBook::new();
        book.apply_snapshot(&[level(52, 500), level(53, 900), level(58, 4_000)], 1);

        assert_eq!(book.depth_to(Prob::from_cents(52).unwrap()), 500);
        assert_eq!(book.depth_to(Prob::from_cents(53).unwrap()), 1_400);
        // Sizing an arb on the top level alone would have understated this
        // one by 1_400 and overstated the price of the rest.
        assert_eq!(book.depth_to(Prob::from_cents(58).unwrap()), 5_400);
    }

    #[test]
    fn a_snapshot_deeper_than_capacity_is_truncated_not_grown() {
        let mut book = OutcomeBook::new();
        let deep: Vec<Level> = (1..=20).map(|i| level(i + 10, 100)).collect();
        book.apply_snapshot(&deep, 1);
        assert_eq!(book.levels().len(), MAX_LEVELS);
    }
}
