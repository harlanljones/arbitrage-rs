//! Integration and performance tests for `arbkit-engine`.

use arbkit_core::book::Level;
use arbkit_core::{Fee, Prob};
use arbkit_engine::{
    spsc_ring, Engine, FeedEventSlot, LatencyHistogram, MarketConfig, SignalEventSlot,
};
use arbkit_feed::{FeedEvent, TapeWriter};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

fn make_level(cents: u32, size: i64) -> Level {
    Level {
        price: Prob::from_cents(cents).unwrap(),
        size,
    }
}

#[test]
fn test_multithreaded_engine_pipeline() {
    let mut engine = Engine::new(32);
    let mut config = MarketConfig {
        active: true,
        outcome_count: 2,
        budget: 100_000,
        ..Default::default()
    };
    config.venue_fees[0] = Fee::None;
    config.venue_fees[1] = Fee::None;
    engine.register_market(0, config).unwrap();

    let (mut feed_prod, feed_cons) = spsc_ring::<FeedEventSlot>(1024);
    let (sig_prod, mut sig_cons) = spsc_ring::<SignalEventSlot>(1024);

    let running = Arc::new(AtomicBool::new(true));
    let running_clone = Arc::clone(&running);

    let engine_handle = thread::spawn(move || {
        let mut sim_clock = 1_000_000u64;
        let clock_fn = move || {
            sim_clock += 100;
            sim_clock
        };
        engine.run(feed_cons, sig_prod, running_clone, clock_fn);
        engine
    });

    // Populate side 0
    feed_prod
        .try_push(FeedEvent::snapshot(
            0,
            0,
            0,
            1,
            1_000_000,
            &[make_level(48, 50_000)],
        ))
        .unwrap();

    // Populate side 1 with arbitrage quote
    feed_prod
        .try_push(FeedEvent::snapshot(
            1,
            0,
            1,
            1,
            1_000_500,
            &[make_level(50, 50_000)],
        ))
        .unwrap();

    // Wait for signal to arrive
    let mut signal_event = None;
    for _ in 0..10_000 {
        if let Some(event) = sig_cons.try_pop() {
            signal_event = Some(event);
            break;
        }
        thread::yield_now();
    }

    assert!(
        signal_event.is_some(),
        "Signal should be emitted to consumer"
    );
    let sig = signal_event.unwrap();
    assert_eq!(sig.market_id, 0);
    assert_eq!(sig.signal.profit_bps, 204);

    running.store(false, Ordering::Relaxed);
    let finished_engine = engine_handle.join().unwrap();
    assert_eq!(finished_engine.stats().signals_emitted, 1);
    assert!(finished_engine.histogram().count() >= 1);
}

#[test]
fn test_stale_book_invalidates_arbitrage() {
    let mut engine = Engine::new(10);
    let config = MarketConfig {
        active: true,
        outcome_count: 2,
        budget: 100_000,
        ..Default::default()
    };
    engine.register_market(0, config).unwrap();

    // Setup active arb
    let event1 = FeedEvent::snapshot(0, 0, 0, 1, 1_000, &[make_level(48, 50_000)]);
    let event2 = FeedEvent::snapshot(1, 0, 1, 1, 2_000, &[make_level(50, 50_000)]);

    assert!(engine.process_event(&event1, 1_500).is_none());
    assert!(engine.process_event(&event2, 2_500).is_some());

    // Invalidate side 0 via Halt event
    let halt_event = FeedEvent::halt(0, 0, Some(0), 3_000, 1);
    assert!(engine.process_event(&halt_event, 3_500).is_none());

    // Update side 1 again - should produce NO signal because side 0 is stale
    let event3 = FeedEvent::snapshot(1, 0, 1, 2, 4_000, &[make_level(49, 50_000)]);
    assert!(engine.process_event(&event3, 4_500).is_none());
}

#[test]
fn test_deterministic_tape_replay() {
    let mut buffer = Vec::new();
    let mut writer = TapeWriter::new(&mut buffer).unwrap();

    // Record a stream of market events
    let event1 = FeedEvent::snapshot(
        0,
        1,
        0,
        1,
        10_000,
        &[make_level(47, 20_000), make_level(49, 30_000)],
    );
    let event2 = FeedEvent::snapshot(1, 1, 1, 1, 20_000, &[make_level(50, 40_000)]);

    writer.write_event(&event1).unwrap();
    writer.write_event(&event2).unwrap();
    writer.flush().unwrap();

    let events = vec![event1, event2];

    // Run replay 1
    let mut engine1 = Engine::new(10);
    let config = MarketConfig {
        active: true,
        outcome_count: 2,
        budget: 50_000,
        ..Default::default()
    };
    engine1.register_market(1, config).unwrap();

    let mut signals1 = Vec::new();
    for event in &events {
        if let Some(sig) = engine1.process_event(event, event.timestamp_ns() + 1_000) {
            signals1.push(sig);
        }
    }

    // Run replay 2
    let mut engine2 = Engine::new(10);
    engine2.register_market(1, config).unwrap();

    let mut signals2 = Vec::new();
    for event in &events {
        if let Some(sig) = engine2.process_event(event, event.timestamp_ns() + 1_000) {
            signals2.push(sig);
        }
    }

    // Signals must be bit-identical across runs
    assert_eq!(signals1.len(), 1);
    assert_eq!(signals1, signals2);
    assert_eq!(engine1.histogram().summary(), engine2.histogram().summary());
}

#[test]
fn test_histogram_resolution_and_quantiles() {
    let mut hist = LatencyHistogram::new();

    // Insert 1000 sub-microsecond samples (100 ns to 1000 ns)
    for i in 1..=1000 {
        hist.record(i);
    }

    assert_eq!(hist.count(), 1000);
    assert!(hist.min_ns().unwrap() <= 10);
    assert!(hist.p50() >= 490 && hist.p50() <= 510);
    assert!(hist.p90() >= 890 && hist.p90() <= 910);
    assert!(hist.p99() >= 980 && hist.p99() <= 1000);
}
