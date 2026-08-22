//! Engine events, signal outputs, and atomic slot definitions for lock-free handoffs.

use crate::ring::AtomicSlot;
use arbkit_core::arb::MAX_CHUNKS;
use arbkit_core::book::{Cents, Level, MarketId, MAX_LEVELS};
use arbkit_core::price::Prob;
use arbkit_core::{Allocation, Fee, OutcomeId, Signal, VenueId};
use arbkit_feed::{FeedEvent, TradeSide};
use std::sync::atomic::{AtomicI64, AtomicU16, AtomicU32, AtomicU64, AtomicU8, Ordering};

/// Tag byte for the [`Fee`] variant carried per plan leg across the ring.
const FEE_TAG_NONE: u8 = 0;
/// Tag byte for `Fee::CommissionBps`.
const FEE_TAG_COMMISSION_BPS: u8 = 1;
/// Tag byte for `Fee::StakeFeeBps`.
const FEE_TAG_STAKE_FEE_BPS: u8 = 2;
/// Tag byte for `Fee::MakerRebateBps`.
const FEE_TAG_MAKER_REBATE_BPS: u8 = 3;

/// Encodes a [`Fee`] into its `(tag, bps argument)` wire form.
fn encode_fee(fee: &Fee) -> (u8, u32) {
    match *fee {
        Fee::None => (FEE_TAG_NONE, 0),
        Fee::CommissionBps(bps) => (FEE_TAG_COMMISSION_BPS, bps),
        Fee::StakeFeeBps(bps) => (FEE_TAG_STAKE_FEE_BPS, bps),
        Fee::MakerRebateBps(bps) => (FEE_TAG_MAKER_REBATE_BPS, bps),
    }
}

/// Decodes the `(tag, bps argument)` wire form back into a [`Fee`].
///
/// Unknown tags degrade to [`Fee::None`] rather than panicking: a corrupted
/// slot must never take down the consumer thread.
fn decode_fee(tag: u8, arg: u32) -> Fee {
    match tag {
        FEE_TAG_COMMISSION_BPS => Fee::CommissionBps(arg),
        FEE_TAG_STAKE_FEE_BPS => Fee::StakeFeeBps(arg),
        FEE_TAG_MAKER_REBATE_BPS => Fee::MakerRebateBps(arg),
        _ => Fee::None,
    }
}

/// A detected arbitrage opportunity enriched with ingestion and processing latency metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignalEvent {
    /// Canonical market identifier.
    pub market_id: MarketId,
    /// Sized and fee-adjusted arbitrage signal.
    pub signal: Signal,
    /// The detection plan this signal was sized against: one entry per
    /// allocation, index-aligned with [`Signal::allocations`]. Carries the
    /// venue, outcome, quote, fee, capacity, and increment each chunk was
    /// staked into, so a downstream simulator can rebuild exact execution
    /// legs instead of guessing at them.
    pub plan: [arbkit_core::Leg; MAX_CHUNKS],
    /// Number of valid entries in [`SignalEvent::plan`].
    pub plan_len: u8,
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
///
/// Carries the full [`MAX_CHUNKS`] allocation width so a multi-chunk plan
/// from `detect_book` crosses the ring intact — truncating here would make
/// the consumer's summed stakes disagree with `total_stake` and silently
/// mis-account every downstream fill. The per-leg plan descriptor travels
/// alongside the signal so consumers rebuild exact execution legs.
///
/// At 8 bytes per atomic field the slot is ~1.1 KiB; the pipeline's
/// 8192-slot signal ring is ~9 MiB, still single-digit MiB territory
/// (verified by `test_slot_footprint_budget` in `ring.rs`).
pub struct SignalEventSlot {
    market_id: AtomicU32,
    len: AtomicU8,
    overround_ppm: AtomicU32,
    total_stake: AtomicI64,
    worst_case_profit: AtomicI64,
    profit_bps: AtomicU32,
    alloc_leg: [AtomicU8; MAX_CHUNKS],
    alloc_stake: [AtomicI64; MAX_CHUNKS],
    alloc_payout: [AtomicI64; MAX_CHUNKS],
    plan_venue: [AtomicU16; MAX_CHUNKS],
    plan_outcome: [AtomicU32; MAX_CHUNKS],
    plan_quoted_ppm: [AtomicU32; MAX_CHUNKS],
    plan_fee_tag: [AtomicU8; MAX_CHUNKS],
    plan_fee_arg_bps: [AtomicU32; MAX_CHUNKS],
    plan_increment: [AtomicI64; MAX_CHUNKS],
    plan_capacity: [AtomicI64; MAX_CHUNKS],
    plan_len: AtomicU8,
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
            plan_venue: Default::default(),
            plan_outcome: Default::default(),
            plan_quoted_ppm: Default::default(),
            plan_fee_tag: Default::default(),
            plan_fee_arg_bps: Default::default(),
            plan_increment: Default::default(),
            plan_capacity: Default::default(),
            plan_len: AtomicU8::new(0),
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
        let count = allocations.len().min(MAX_CHUNKS);
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

        let plan_len = (item.plan_len as usize).min(MAX_CHUNKS);
        for i in 0..plan_len {
            let leg = &item.plan[i];
            let (fee_tag, fee_arg) = encode_fee(&leg.fee);
            self.plan_venue[i].store(leg.venue, Ordering::Relaxed);
            self.plan_outcome[i].store(leg.outcome, Ordering::Relaxed);
            self.plan_quoted_ppm[i].store(leg.quoted.ppm(), Ordering::Relaxed);
            self.plan_fee_tag[i].store(fee_tag, Ordering::Relaxed);
            self.plan_fee_arg_bps[i].store(fee_arg, Ordering::Relaxed);
            self.plan_increment[i].store(leg.increment, Ordering::Relaxed);
            self.plan_capacity[i].store(leg.capacity, Ordering::Relaxed);
        }
        self.plan_len.store(plan_len as u8, Ordering::Relaxed);

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
        }; MAX_CHUNKS];
        let count = (len as usize).min(MAX_CHUNKS);
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

        let plan_len = self.plan_len.load(Ordering::Relaxed);
        let mut plan = [arbkit_core::Leg {
            venue: 0 as VenueId,
            outcome: 0 as OutcomeId,
            quoted: Prob::CERTAIN,
            fee: Fee::None,
            capacity: 0 as Cents,
            increment: 1 as Cents,
        }; MAX_CHUNKS];
        for (i, leg) in plan.iter_mut().take(plan_len as usize).enumerate() {
            *leg = arbkit_core::Leg {
                venue: self.plan_venue[i].load(Ordering::Relaxed),
                outcome: self.plan_outcome[i].load(Ordering::Relaxed),
                quoted: Prob::from_ppm(self.plan_quoted_ppm[i].load(Ordering::Relaxed))
                    .unwrap_or(Prob::CERTAIN),
                fee: decode_fee(
                    self.plan_fee_tag[i].load(Ordering::Relaxed),
                    self.plan_fee_arg_bps[i].load(Ordering::Relaxed),
                ),
                capacity: self.plan_capacity[i].load(Ordering::Relaxed),
                increment: self.plan_increment[i].load(Ordering::Relaxed),
            };
        }

        let ingest_timestamp_ns = self.ingest_timestamp_ns.load(Ordering::Relaxed);
        let signal_timestamp_ns = self.signal_timestamp_ns.load(Ordering::Relaxed);
        let latency_ns = self.latency_ns.load(Ordering::Relaxed);

        SignalEvent {
            market_id,
            signal,
            plan,
            plan_len,
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
        let mut allocs = [Allocation {
            leg: 0,
            stake: 0,
            payout: 0,
        }; MAX_CHUNKS];
        allocs[0] = Allocation {
            leg: 0,
            stake: 50_000,
            payout: 102_000,
        };
        allocs[1] = Allocation {
            leg: 1,
            stake: 49_999,
            payout: 102_000,
        };
        let signal = Signal::from_raw_parts(allocs, 2, 980_000, 99_999, 2_001, 200);

        let event = SignalEvent {
            market_id: 1,
            signal,
            plan: [arbkit_core::Leg {
                venue: 0,
                outcome: 0,
                quoted: Prob::from_cents(48).unwrap(),
                fee: Fee::None,
                capacity: 50_000,
                increment: 1,
            }; MAX_CHUNKS],
            plan_len: 2,
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
        assert_eq!(loaded.plan_len, 2);
        assert_eq!(loaded.plan[0].venue, 0);
        assert_eq!(
            loaded.plan[0].quoted.ppm(),
            Prob::from_cents(48).unwrap().ppm()
        );
    }

    /// A plan wider than the old one-allocation-per-outcome wire must cross
    /// the ring with every chunk intact; truncation here is what would make
    /// downstream accounting disagree with `total_stake`.
    #[test]
    fn test_signal_event_slot_carries_max_chunk_plan() {
        let slot = SignalEventSlot::default();
        let mut allocs = [Allocation {
            leg: 0,
            stake: 0,
            payout: 0,
        }; MAX_CHUNKS];
        for (i, alloc) in allocs.iter_mut().enumerate() {
            *alloc = Allocation {
                leg: i % 4,
                stake: (i as i64 + 1) * 100,
                payout: (i as i64 + 1) * 210,
            };
        }
        let total: i64 = allocs.iter().map(|a| a.stake).sum();
        let signal = Signal::from_raw_parts(allocs, MAX_CHUNKS as u8, 970_000, total, 500, 50);

        let event = SignalEvent {
            market_id: 7,
            signal,
            plan: allocs
                .iter()
                .map(|a| arbkit_core::Leg {
                    venue: (a.leg % 2) as u16,
                    outcome: (a.leg % 4) as u32,
                    quoted: Prob::from_cents(50).unwrap(),
                    fee: Fee::StakeFeeBps(350),
                    capacity: a.stake,
                    increment: 1,
                })
                .collect::<Vec<_>>()
                .try_into()
                .unwrap(),
            plan_len: MAX_CHUNKS as u8,
            ingest_timestamp_ns: 10,
            signal_timestamp_ns: 20,
            latency_ns: 10,
        };

        slot.store(&event);
        let loaded = slot.load().signal;
        assert_eq!(loaded.allocations().len(), MAX_CHUNKS);
        assert_eq!(loaded.total_stake, total);
        for (a, b) in loaded.allocations().iter().zip(allocs.iter()) {
            assert_eq!(a, b);
        }
    }

    /// The fee codec must round-trip every variant; a degraded fee silently
    /// mis-prices fills downstream.
    #[test]
    fn test_fee_codec_roundtrip() {
        let fees = [
            Fee::None,
            Fee::CommissionBps(175),
            Fee::StakeFeeBps(350),
            Fee::MakerRebateBps(200),
        ];
        for fee in fees {
            let (tag, arg) = encode_fee(&fee);
            assert_eq!(decode_fee(tag, arg), fee);
        }
        // Unknown tags fail safe to no-fee, never panic.
        assert_eq!(decode_fee(200, 7), Fee::None);
    }
}
