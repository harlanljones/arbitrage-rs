//! The hot loop execution engine.
//!
//! Orchestrates ingestion from the SPSC ring buffer, applies book updates to the
//! preallocated slab, triggers market aggregation, records latency, and emits signals.

use crate::aggregator::Aggregator;
use crate::error::Result;
use crate::event::{FeedEventSlot, SignalEvent, SignalEventSlot};
use crate::histogram::LatencyHistogram;
use crate::ring::{Consumer, Producer};
use crate::slab::{EngineSlab, MarketConfig, DEFAULT_MAX_MARKETS, MAX_OUTCOMES};
use arbkit_core::book::{MarketId, OutcomeId, MAX_LEVELS};
use arbkit_feed::FeedEvent;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Operational metrics and statistics collected by the engine hot loop.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EngineStats {
    /// Total feed events processed.
    pub events_processed: u64,
    /// Total arbitrage signals detected and emitted.
    pub signals_emitted: u64,
    /// Total order book snapshots processed.
    pub snapshots_processed: u64,
    /// Total incremental deltas processed.
    pub deltas_processed: u64,
    /// Total trade executions processed.
    pub trades_processed: u64,
    /// Total trading halts/suspensions processed.
    pub halts_processed: u64,
    /// Total heartbeats processed.
    pub heartbeats_processed: u64,
}

/// The single-threaded hot path arbitrage engine.
#[derive(Debug)]
pub struct Engine {
    slab: EngineSlab,
    histogram: LatencyHistogram,
    stats: EngineStats,
}

impl Default for Engine {
    fn default() -> Self {
        Self::with_default_capacity()
    }
}

impl Engine {
    /// Creates a new engine with preallocated storage for `max_markets`.
    pub fn new(max_markets: usize) -> Self {
        Self {
            slab: EngineSlab::new(max_markets),
            histogram: LatencyHistogram::new(),
            stats: EngineStats::default(),
        }
    }

    /// Creates a new engine using the default preallocated market capacity.
    pub fn with_default_capacity() -> Self {
        Self::new(DEFAULT_MAX_MARKETS)
    }

    /// Registers a market's parameters and fee configuration.
    pub fn register_market(&mut self, market_id: MarketId, config: MarketConfig) -> Result<()> {
        self.slab.register_market(market_id, config)
    }

    /// Processes a single [`FeedEvent`], updating the book and evaluating arbitrage.
    ///
    /// Records the elapsed service latency (ingest timestamp to `emit_timestamp_ns`) in
    /// the engine histogram for *every* event processed here, signal or not — see the
    /// `histogram` module docs for why this covers all events rather than only
    /// signal-emitting ones. If an arbitrage opportunity is detected, returns
    /// `Some(SignalEvent)` and increments `stats.signals_emitted`, which lets callers
    /// recover the old signal-hits-only rate independently of the histogram.
    #[inline]
    pub fn process_event(
        &mut self,
        event: &FeedEvent,
        emit_timestamp_ns: u64,
    ) -> Option<SignalEvent> {
        self.stats.events_processed += 1;

        let ingest_timestamp_ns = event.timestamp_ns();
        let latency_ns = emit_timestamp_ns.saturating_sub(ingest_timestamp_ns);
        self.histogram.record(latency_ns);

        match event {
            FeedEvent::Snapshot {
                venue_id,
                market_id,
                outcome_id,
                seq,
                levels,
                num_levels,
                ..
            } => {
                self.stats.snapshots_processed += 1;
                if let Some(book) = self.slab.get_book_mut(*market_id, *outcome_id, *venue_id) {
                    let take = (*num_levels as usize).min(MAX_LEVELS);
                    book.apply_snapshot(&levels[..take], *seq);
                }
            }
            FeedEvent::Delta {
                venue_id,
                market_id,
                outcome_id,
                seq,
                level,
                is_delete,
                ..
            } => {
                self.stats.deltas_processed += 1;
                if let Some(book) = self.slab.get_book_mut(*market_id, *outcome_id, *venue_id) {
                    if book.accept_seq(*seq) {
                        if *is_delete {
                            book.apply_snapshot(&[], *seq);
                        } else {
                            book.apply_snapshot(&[*level], *seq);
                        }
                    }
                }
            }
            FeedEvent::Trade { .. } => {
                self.stats.trades_processed += 1;
                return None;
            }
            FeedEvent::Halt {
                venue_id,
                market_id,
                outcome_id,
                ..
            } => {
                self.stats.halts_processed += 1;
                if let Some(oid) = outcome_id {
                    if let Some(book) = self.slab.get_book_mut(*market_id, *oid, *venue_id) {
                        book.mark_stale();
                    }
                } else {
                    for o in 0..MAX_OUTCOMES {
                        if let Some(book) =
                            self.slab
                                .get_book_mut(*market_id, o as OutcomeId, *venue_id)
                        {
                            book.mark_stale();
                        }
                    }
                }
            }
            FeedEvent::Heartbeat { .. } => {
                self.stats.heartbeats_processed += 1;
                return None;
            }
        }

        if let Some(market_id) = event.market_id() {
            // Cooldown gate runs before detection's result is trusted for
            // emission: a suppressed duplicate must not extend its own
            // window, so admission is read first and only an emitted
            // signal opens (or reopens) the market's cooldown.
            if !self.slab.emit_admitted(market_id, emit_timestamp_ns) {
                return None;
            }
            if let Ok(Some((signal, plan, plan_len))) =
                Aggregator::evaluate_market(&self.slab, market_id)
            {
                self.slab.note_emit(market_id, emit_timestamp_ns);
                self.stats.signals_emitted += 1;

                return Some(SignalEvent {
                    market_id,
                    signal,
                    plan,
                    plan_len,
                    ingest_timestamp_ns,
                    signal_timestamp_ns: emit_timestamp_ns,
                    latency_ns,
                });
            }
        }

        None
    }

    /// Performs a non-blocking single step of the hot loop.
    ///
    /// Reads an event from the input queue, processes it, and emits any generated
    /// signal to the output queue. Returns `true` if an event was processed.
    #[inline]
    pub fn step(
        &mut self,
        input: &mut Consumer<FeedEventSlot>,
        output: &mut Producer<SignalEventSlot>,
        mut clock_fn: impl FnMut() -> u64,
    ) -> bool {
        if let Some(event) = input.try_pop() {
            let now_ns = clock_fn();
            if let Some(signal_event) = self.process_event(&event, now_ns) {
                let _ = output.try_push(signal_event);
            }
            true
        } else {
            false
        }
    }

    /// Runs the hot loop continuously on the current thread until `running` is cleared.
    pub fn run(
        &mut self,
        mut input: Consumer<FeedEventSlot>,
        mut output: Producer<SignalEventSlot>,
        running: Arc<AtomicBool>,
        mut clock_fn: impl FnMut() -> u64,
    ) {
        while running.load(Ordering::Relaxed) {
            if !self.step(&mut input, &mut output, &mut clock_fn) {
                std::hint::spin_loop();
            }
        }
    }

    /// Returns a reference to the preallocated order book slab.
    #[inline]
    pub fn slab(&self) -> &EngineSlab {
        &self.slab
    }

    /// Returns a mutable reference to the preallocated order book slab.
    #[inline]
    pub fn slab_mut(&mut self) -> &mut EngineSlab {
        &mut self.slab
    }

    /// Returns a reference to the latency histogram.
    #[inline]
    pub fn histogram(&self) -> &LatencyHistogram {
        &self.histogram
    }

    /// Returns a reference to the engine operational statistics.
    #[inline]
    pub fn stats(&self) -> &EngineStats {
        &self.stats
    }

    /// Resets the engine state, clearing books, statistics, and latency metrics.
    pub fn reset(&mut self) {
        self.slab.reset();
        self.histogram.reset();
        self.stats = EngineStats::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ring::spsc_ring;
    use arbkit_core::book::Level;
    use arbkit_core::Prob;

    #[test]
    fn test_engine_end_to_end_signal_generation() {
        let mut engine = Engine::new(10);
        let config = MarketConfig {
            active: true,
            outcome_count: 2,
            budget: 100_000,
            ..Default::default()
        };
        engine.register_market(0, config).unwrap();

        let (mut feed_prod, mut feed_cons) = spsc_ring::<FeedEventSlot>(64);
        let (mut sig_prod, mut sig_cons) = spsc_ring::<SignalEventSlot>(64);

        // Send Snapshot for outcome 0 on venue 0: 48c
        feed_prod
            .try_push(FeedEvent::snapshot(
                0,
                0,
                0,
                1,
                1_000_000,
                &[Level {
                    price: Prob::from_cents(48).unwrap(),
                    size: 50_000,
                }],
            ))
            .unwrap();

        // Step engine (only 1 leg populated so far, no signal yet)
        assert!(engine.step(&mut feed_cons, &mut sig_prod, || 1_001_000));
        assert!(sig_cons.try_pop().is_none());

        // Send Snapshot for outcome 1 on venue 1: 50c
        feed_prod
            .try_push(FeedEvent::snapshot(
                1,
                0,
                1,
                1,
                1_002_000,
                &[Level {
                    price: Prob::from_cents(50).unwrap(),
                    size: 50_000,
                }],
            ))
            .unwrap();

        // Step engine (both legs present, arbitrage detected!)
        assert!(engine.step(&mut feed_cons, &mut sig_prod, || 1_005_000));

        let signal_event = sig_cons.try_pop().expect("signal should be emitted");
        assert_eq!(signal_event.market_id, 0);
        assert_eq!(signal_event.signal.profit_bps, 204);
        assert_eq!(signal_event.ingest_timestamp_ns, 1_002_000);
        assert_eq!(signal_event.signal_timestamp_ns, 1_005_000);
        assert_eq!(signal_event.latency_ns, 3_000);

        assert_eq!(engine.stats().events_processed, 2);
        assert_eq!(engine.stats().signals_emitted, 1);
        // The histogram now records service time for every processed event, not just
        // the one that emitted a signal, so both steps contribute a sample.
        assert_eq!(engine.histogram().count(), 2);
        assert_eq!(engine.histogram().min_ns(), Some(1_000));
        assert_eq!(engine.histogram().max_ns(), Some(3_000));
    }

    /// Every processed event — whether or not it produces a signal — must contribute
    /// exactly one histogram sample, and the signal-emitted counter must track actual
    /// signals independently of the histogram sample count.
    #[test]
    fn test_histogram_records_every_event_not_just_signal_hits() {
        let mut engine = Engine::new(10);
        let config = MarketConfig {
            active: true,
            outcome_count: 2,
            budget: 100_000,
            ..Default::default()
        };
        engine.register_market(0, config).unwrap();

        let (mut feed_prod, mut feed_cons) = spsc_ring::<FeedEventSlot>(64);
        let (mut sig_prod, mut sig_cons) = spsc_ring::<SignalEventSlot>(64);

        // A heartbeat: no market, never emits a signal, must still record a sample.
        feed_prod
            .try_push(FeedEvent::Heartbeat {
                venue_id: 0,
                timestamp_ns: 10,
            })
            .unwrap();
        assert!(engine.step(&mut feed_cons, &mut sig_prod, || 20));
        assert!(sig_cons.try_pop().is_none());
        assert_eq!(engine.histogram().count(), 1);
        assert_eq!(engine.stats().signals_emitted, 0);

        // A snapshot with only one leg populated: no signal yet, must still record.
        feed_prod
            .try_push(FeedEvent::snapshot(
                0,
                0,
                0,
                1,
                1_000,
                &[Level {
                    price: Prob::from_cents(48).unwrap(),
                    size: 50_000,
                }],
            ))
            .unwrap();
        assert!(engine.step(&mut feed_cons, &mut sig_prod, || 1_100));
        assert!(sig_cons.try_pop().is_none());
        assert_eq!(engine.histogram().count(), 2);
        assert_eq!(engine.stats().signals_emitted, 0);

        // A snapshot completing the arbitrage: this one does emit a signal, and must
        // still only contribute a single histogram sample.
        feed_prod
            .try_push(FeedEvent::snapshot(
                1,
                0,
                1,
                1,
                2_000,
                &[Level {
                    price: Prob::from_cents(50).unwrap(),
                    size: 50_000,
                }],
            ))
            .unwrap();
        assert!(engine.step(&mut feed_cons, &mut sig_prod, || 5_000));
        assert!(sig_cons.try_pop().is_some());
        assert_eq!(engine.histogram().count(), 3);
        assert_eq!(engine.stats().signals_emitted, 1);
        assert_eq!(engine.stats().events_processed, 3);
    }

    /// Malformed/edge-case events (unregistered markets, out-of-range ids, zero-level
    /// snapshots, halts with no outcome, timestamps after the emit clock) must never
    /// panic on `process_event`, mirroring `arbkit-core`'s `detection_is_total`
    /// property: `Ok(None)`/`None` is always an acceptable outcome, a panic never is.
    #[test]
    fn test_process_event_never_panics_on_edge_cases() {
        let mut engine = Engine::new(2);
        let config = MarketConfig {
            active: true,
            outcome_count: 2,
            budget: 100_000,
            ..Default::default()
        };
        engine.register_market(0, config).unwrap();

        let edge_events = [
            // Unregistered market id.
            FeedEvent::snapshot(0, 99, 0, 1, 0, &[]),
            // Out-of-range outcome id on a registered market.
            FeedEvent::snapshot(0, 0, 250, 1, 0, &[]),
            // Zero-level snapshot.
            FeedEvent::snapshot(0, 0, 0, 1, 0, &[]),
            // Delta with a stale/duplicate sequence number.
            FeedEvent::Delta {
                venue_id: 0,
                market_id: 0,
                outcome_id: 0,
                seq: 0,
                level: Level {
                    price: Prob::from_cents(50).unwrap(),
                    size: 1,
                },
                is_delete: false,
                timestamp_ns: 0,
            },
            // Halt with no specific outcome, across an unregistered market.
            FeedEvent::Halt {
                venue_id: 0,
                market_id: 42,
                outcome_id: None,
                timestamp_ns: 0,
                reason_code: 0,
            },
            // Heartbeat.
            FeedEvent::Heartbeat {
                venue_id: 0,
                timestamp_ns: 0,
            },
            // Emit timestamp before ingest timestamp (clock skew): latency must
            // saturate rather than underflow/panic.
            FeedEvent::snapshot(
                0,
                0,
                0,
                2,
                u64::MAX,
                &[Level {
                    price: Prob::from_cents(48).unwrap(),
                    size: 1,
                }],
            ),
        ];

        for event in &edge_events {
            let _ = engine.process_event(event, 0);
        }

        assert_eq!(engine.stats().events_processed, edge_events.len() as u64);
        assert_eq!(engine.histogram().count(), edge_events.len() as u64);
    }
}
