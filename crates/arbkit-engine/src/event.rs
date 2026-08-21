//! Engine events, signal outputs, and atomic slot definitions for lock-free handoffs.

use crate::ring::AtomicSlot;
use arbkit_core::book::{Level, MarketId, MAX_LEVELS};
use arbkit_core::price::Prob;
use arbkit_core::{Allocation, Signal, MAX_LEGS};
use arbkit_feed::{FeedEvent, TradeSide};
use std::sync::atomic::{AtomicI64, AtomicU16, AtomicU32, AtomicU64, AtomicU8, Ordering};

/// A detected arbitrage opportunity enriched with ingestion and processing latency metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignalEvent {
    /// Canonical market identifier.
    pub market_id: MarketId,
    /// Sized and fee-adjusted arbitrage signal.
    pub signal: Signal,
    /// Ingestion timestamp in nanoseconds when triggering feed event arrived.
    pub ingest_timestamp_ns: u64,
    /// Emission timestamp in nanoseconds when engine generated this signal.
    pub signal_timestamp_ns: u64,
    /// Total elapsed latency in nanoseconds (`signal_timestamp_ns - ingest_timestamp_ns`).
    pub latency_ns: u64,
}

/// Atomic slot representation of [`FeedEvent`] for lock-free transmission.
pub struct FeedEventSlot {
    kind: AtomicU8,
    market_id: AtomicU32,
    outcome_id: AtomicU32,
    venue_id: AtomicU16,
    num_levels: AtomicU8,
    seq: AtomicU64,
    timestamp_ns: AtomicU64,
    level_prices: [AtomicU32; MAX_LEVELS],
    level_sizes: [AtomicI64; MAX_LEVELS],
    extra_flags: AtomicU8,
}

impl Default for FeedEventSlot {
    fn default() -> Self {
        Self {
            kind: AtomicU8::new(0),
            market_id: AtomicU32::new(0),
            outcome_id: AtomicU32::new(0),
            venue_id: AtomicU16::new(0),
            num_levels: AtomicU8::new(0),
            seq: AtomicU64::new(0),
            timestamp_ns: AtomicU64::new(0),
            level_prices: Default::default(),
            level_sizes: Default::default(),
            extra_flags: AtomicU8::new(0),
        }
    }
}

impl AtomicSlot for FeedEventSlot {
    type Item = FeedEvent;

    fn store(&self, item: &Self::Item) {
        match item {
            FeedEvent::Snapshot {
                venue_id,
                market_id,
                outcome_id,
                seq,
                timestamp_ns,
                levels,
                num_levels,
            } => {
                self.venue_id.store(*venue_id, Ordering::Relaxed);
                self.market_id.store(*market_id, Ordering::Relaxed);
                self.outcome_id.store(*outcome_id, Ordering::Relaxed);
                self.num_levels.store(*num_levels, Ordering::Relaxed);
                self.seq.store(*seq, Ordering::Relaxed);
                self.timestamp_ns.store(*timestamp_ns, Ordering::Relaxed);
                let count = (*num_levels as usize).min(MAX_LEVELS);
                for (i, level) in levels.iter().take(count).enumerate() {
                    self.level_prices[i].store(level.price.ppm(), Ordering::Relaxed);
                    self.level_sizes[i].store(level.size, Ordering::Relaxed);
                }
                self.extra_flags.store(0, Ordering::Relaxed);
                self.kind.store(1, Ordering::Relaxed);
            }
            FeedEvent::Delta {
                venue_id,
                market_id,
                outcome_id,
                seq,
                timestamp_ns,
                level,
                is_delete,
            } => {
                self.venue_id.store(*venue_id, Ordering::Relaxed);
                self.market_id.store(*market_id, Ordering::Relaxed);
                self.outcome_id.store(*outcome_id, Ordering::Relaxed);
                self.num_levels.store(1, Ordering::Relaxed);
                self.seq.store(*seq, Ordering::Relaxed);
                self.timestamp_ns.store(*timestamp_ns, Ordering::Relaxed);
                self.level_prices[0].store(level.price.ppm(), Ordering::Relaxed);
                self.level_sizes[0].store(level.size, Ordering::Relaxed);
                self.extra_flags
                    .store(if *is_delete { 1 } else { 0 }, Ordering::Relaxed);
                self.kind.store(2, Ordering::Relaxed);
            }
            FeedEvent::Trade {
                venue_id,
                market_id,
                outcome_id,
                seq,
                timestamp_ns,
                price,
                size,
                side,
            } => {
                self.venue_id.store(*venue_id, Ordering::Relaxed);
                self.market_id.store(*market_id, Ordering::Relaxed);
                self.outcome_id.store(*outcome_id, Ordering::Relaxed);
                self.num_levels.store(1, Ordering::Relaxed);
                self.seq.store(*seq, Ordering::Relaxed);
                self.timestamp_ns.store(*timestamp_ns, Ordering::Relaxed);
                self.level_prices[0].store(price.ppm(), Ordering::Relaxed);
                self.level_sizes[0].store(*size, Ordering::Relaxed);
                self.extra_flags.store(side.to_u8(), Ordering::Relaxed);
                self.kind.store(3, Ordering::Relaxed);
            }
            FeedEvent::Heartbeat {
                venue_id,
                timestamp_ns,
            } => {
                self.venue_id.store(*venue_id, Ordering::Relaxed);
                self.timestamp_ns.store(*timestamp_ns, Ordering::Relaxed);
                self.kind.store(4, Ordering::Relaxed);
            }
            FeedEvent::Halt {
                venue_id,
                market_id,
                outcome_id,
                timestamp_ns,
                reason_code,
            } => {
                self.venue_id.store(*venue_id, Ordering::Relaxed);
                self.market_id.store(*market_id, Ordering::Relaxed);
                if let Some(oid) = outcome_id {
                    self.outcome_id.store(*oid, Ordering::Relaxed);
                    self.extra_flags.store(1, Ordering::Relaxed);
                } else {
                    self.outcome_id.store(0, Ordering::Relaxed);
                    self.extra_flags.store(0, Ordering::Relaxed);
                }
                self.num_levels.store(*reason_code, Ordering::Relaxed);
                self.timestamp_ns.store(*timestamp_ns, Ordering::Relaxed);
                self.kind.store(5, Ordering::Relaxed);
            }
        }
    }

    fn load(&self) -> Self::Item {
        let kind = self.kind.load(Ordering::Relaxed);
        let timestamp_ns = self.timestamp_ns.load(Ordering::Relaxed);
        let venue_id = self.venue_id.load(Ordering::Relaxed);
        let market_id = self.market_id.load(Ordering::Relaxed);
        let outcome_id = self.outcome_id.load(Ordering::Relaxed);
        let seq = self.seq.load(Ordering::Relaxed);

        match kind {
            1 => {
                let num_levels = self.num_levels.load(Ordering::Relaxed);
                let mut levels = [Level {
                    price: Prob::CERTAIN,
                    size: 0,
                }; MAX_LEVELS];
                let count = (num_levels as usize).min(MAX_LEVELS);
                for (i, level) in levels.iter_mut().take(count).enumerate() {
                    let ppm = self.level_prices[i].load(Ordering::Relaxed);
                    let size = self.level_sizes[i].load(Ordering::Relaxed);
                    *level = Level {
                        price: Prob::from_ppm(ppm).unwrap_or(Prob::CERTAIN),
                        size,
                    };
                }
                FeedEvent::Snapshot {
                    venue_id,
                    market_id,
                    outcome_id,
                    seq,
                    timestamp_ns,
                    levels,
                    num_levels,
                }
            }
            2 => {
                let ppm = self.level_prices[0].load(Ordering::Relaxed);
                let size = self.level_sizes[0].load(Ordering::Relaxed);
                let is_delete = self.extra_flags.load(Ordering::Relaxed) != 0;
                FeedEvent::Delta {
                    venue_id,
                    market_id,
                    outcome_id,
                    seq,
                    timestamp_ns,
                    level: Level {
                        price: Prob::from_ppm(ppm).unwrap_or(Prob::CERTAIN),
                        size,
                    },
                    is_delete,
                }
            }
            3 => {
                let ppm = self.level_prices[0].load(Ordering::Relaxed);
                let size = self.level_sizes[0].load(Ordering::Relaxed);
                let side = TradeSide::from_u8(self.extra_flags.load(Ordering::Relaxed));
                FeedEvent::Trade {
                    venue_id,
                    market_id,
                    outcome_id,
                    seq,
                    timestamp_ns,
                    price: Prob::from_ppm(ppm).unwrap_or(Prob::CERTAIN),
                    size,
                    side,
                }
            }
            5 => {
                let has_outcome = self.extra_flags.load(Ordering::Relaxed) != 0;
                let outcome_id = if has_outcome { Some(outcome_id) } else { None };
                let reason_code = self.num_levels.load(Ordering::Relaxed);
                FeedEvent::Halt {
                    venue_id,
                    market_id,
                    outcome_id,
                    timestamp_ns,
                    reason_code,
                }
            }
            _ => FeedEvent::Heartbeat {
                venue_id,
                timestamp_ns,
            },
        }
    }
}

/// Atomic slot representation of [`SignalEvent`] for lock-free handoffs.
pub struct SignalEventSlot {
    market_id: AtomicU32,
    len: AtomicU8,
    overround_ppm: AtomicU32,
    total_stake: AtomicI64,
    worst_case_profit: AtomicI64,
    profit_bps: AtomicU32,
    alloc_leg: [AtomicU8; MAX_LEGS],
    alloc_stake: [AtomicI64; MAX_LEGS],
    alloc_payout: [AtomicI64; MAX_LEGS],
    ingest_timestamp_ns: AtomicU64,
    signal_timestamp_ns: AtomicU64,
    latency_ns: AtomicU64,
}

impl Default for SignalEventSlot {
    fn default() -> Self {
        Self {
            market_id: AtomicU32::new(0),
            len: AtomicU8::new(0),
            overround_ppm: AtomicU32::new(0),
            total_stake: AtomicI64::new(0),
            worst_case_profit: AtomicI64::new(0),
            profit_bps: AtomicU32::new(0),
            alloc_leg: Default::default(),
            alloc_stake: Default::default(),
            alloc_payout: Default::default(),
            ingest_timestamp_ns: AtomicU64::new(0),
            signal_timestamp_ns: AtomicU64::new(0),
            latency_ns: AtomicU64::new(0),
        }
    }
}

impl AtomicSlot for SignalEventSlot {
    type Item = SignalEvent;

    fn store(&self, item: &Self::Item) {
        self.market_id.store(item.market_id, Ordering::Relaxed);
        let allocations = item.signal.allocations();
        let count = allocations.len().min(MAX_LEGS);
        self.len.store(count as u8, Ordering::Relaxed);
        self.overround_ppm
            .store(item.signal.overround_ppm, Ordering::Relaxed);
        self.total_stake
            .store(item.signal.total_stake, Ordering::Relaxed);
        self.worst_case_profit
            .store(item.signal.worst_case_profit, Ordering::Relaxed);
        self.profit_bps
            .store(item.signal.profit_bps, Ordering::Relaxed);

        for (i, alloc) in allocations.iter().take(count).enumerate() {
            self.alloc_leg[i].store(alloc.leg as u8, Ordering::Relaxed);
            self.alloc_stake[i].store(alloc.stake, Ordering::Relaxed);
            self.alloc_payout[i].store(alloc.payout, Ordering::Relaxed);
        }

        self.ingest_timestamp_ns
            .store(item.ingest_timestamp_ns, Ordering::Relaxed);
        self.signal_timestamp_ns
            .store(item.signal_timestamp_ns, Ordering::Relaxed);
        self.latency_ns.store(item.latency_ns, Ordering::Relaxed);
    }

    fn load(&self) -> Self::Item {
        let market_id = self.market_id.load(Ordering::Relaxed);
        let len = self.len.load(Ordering::Relaxed);
        let overround_ppm = self.overround_ppm.load(Ordering::Relaxed);
        let total_stake = self.total_stake.load(Ordering::Relaxed);
        let worst_case_profit = self.worst_case_profit.load(Ordering::Relaxed);
        let profit_bps = self.profit_bps.load(Ordering::Relaxed);

        let mut allocations = [Allocation {
            leg: 0,
            stake: 0,
            payout: 0,
        }; MAX_LEGS];
        let count = (len as usize).min(MAX_LEGS);
        for (i, alloc) in allocations.iter_mut().take(count).enumerate() {
            *alloc = Allocation {
                leg: self.alloc_leg[i].load(Ordering::Relaxed) as usize,
                stake: self.alloc_stake[i].load(Ordering::Relaxed),
                payout: self.alloc_payout[i].load(Ordering::Relaxed),
            };
        }

        let signal = Signal::from_raw_parts(
            allocations,
            len,
            overround_ppm,
            total_stake,
            worst_case_profit,
            profit_bps,
        );

        let ingest_timestamp_ns = self.ingest_timestamp_ns.load(Ordering::Relaxed);
        let signal_timestamp_ns = self.signal_timestamp_ns.load(Ordering::Relaxed);
        let latency_ns = self.latency_ns.load(Ordering::Relaxed);

        SignalEvent {
            market_id,
            signal,
            ingest_timestamp_ns,
            signal_timestamp_ns,
            latency_ns,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feed_event_slot_roundtrip() {
        let slot = FeedEventSlot::default();
        let event = FeedEvent::delta(
            3,
            12,
            1,
            42,
            1234567,
            Level {
                price: Prob::from_cents(48).unwrap(),
                size: 5_000,
            },
            false,
        );

        slot.store(&event);
        let loaded = slot.load();
        assert_eq!(loaded, event);
    }

    #[test]
    fn test_signal_event_slot_roundtrip() {
        let slot = SignalEventSlot::default();
        let allocs = [
            Allocation {
                leg: 0,
                stake: 50_000,
                payout: 102_000,
            },
            Allocation {
                leg: 1,
                stake: 49_999,
                payout: 102_000,
            },
            Allocation {
                leg: 0,
                stake: 0,
                payout: 0,
            },
            Allocation {
                leg: 0,
                stake: 0,
                payout: 0,
            },
        ];
        let signal = Signal::from_raw_parts(allocs, 2, 980_000, 99_999, 2_001, 200);

        let event = SignalEvent {
            market_id: 1,
            signal,
            ingest_timestamp_ns: 1_000_000,
            signal_timestamp_ns: 1_005_000,
            latency_ns: 5_000,
        };

        slot.store(&event);
        let loaded = slot.load();
        assert_eq!(loaded.market_id, event.market_id);
        assert_eq!(loaded.latency_ns, 5_000);
        assert_eq!(loaded.signal.profit_bps, 200);
        assert_eq!(loaded.signal.allocations().len(), 2);
    }
}
