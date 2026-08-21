//! Property tests for `arbkit-feed` encoding, decoding, and price normalization.

use arbkit_core::{Level, Prob, MAX_LEVELS, PPM};
use arbkit_feed::tape::format::{decode_event, encode_event, MAX_RECORD_SIZE};
use arbkit_feed::{parse_decimal_prob, FeedEvent, TradeSide};
use proptest::prelude::*;

fn arb_prob() -> impl Strategy<Value = Prob> {
    (1..=PPM).prop_map(|ppm| Prob::from_ppm(ppm).unwrap())
}

fn arb_level() -> impl Strategy<Value = Level> {
    (arb_prob(), 0..1_000_000_000i64).prop_map(|(price, size)| Level { price, size })
}

fn arb_trade_side() -> impl Strategy<Value = TradeSide> {
    prop_oneof![
        Just(TradeSide::Unknown),
        Just(TradeSide::Buy),
        Just(TradeSide::Sell),
    ]
}

fn arb_feed_event() -> impl Strategy<Value = FeedEvent> {
    prop_oneof![
        // Snapshot
        (
            any::<u16>(),
            any::<u32>(),
            any::<u32>(),
            any::<u64>(),
            any::<u64>(),
            prop::collection::vec(arb_level(), 0..=MAX_LEVELS)
        )
            .prop_map(|(venue, market, outcome, seq, ts, levels)| {
                FeedEvent::snapshot(venue, market, outcome, seq, ts, &levels)
            }),
        // Delta
        (
            any::<u16>(),
            any::<u32>(),
            any::<u32>(),
            any::<u64>(),
            any::<u64>(),
            arb_level(),
            any::<bool>()
        )
            .prop_map(|(venue, market, outcome, seq, ts, level, is_delete)| {
                FeedEvent::delta(venue, market, outcome, seq, ts, level, is_delete)
            }),
        // Trade
        (
            any::<u16>(),
            any::<u32>(),
            any::<u32>(),
            any::<u64>(),
            any::<u64>(),
            arb_prob(),
            0..1_000_000_000i64,
            arb_trade_side()
        )
            .prop_map(|(venue, market, outcome, seq, ts, price, size, side)| {
                FeedEvent::trade(venue, market, outcome, seq, ts, price, size, side)
            }),
        // Heartbeat
        (any::<u16>(), any::<u64>()).prop_map(|(venue, ts)| FeedEvent::heartbeat(venue, ts)),
        // Halt
        (
            any::<u16>(),
            any::<u32>(),
            prop::option::of(any::<u32>()),
            any::<u64>(),
            any::<u8>()
        )
            .prop_map(|(venue, market, outcome, ts, reason)| {
                FeedEvent::halt(venue, market, outcome, ts, reason)
            }),
    ]
}

proptest! {
    #[test]
    fn prop_feed_event_binary_roundtrip(event in arb_feed_event()) {
        let mut buf = [0u8; MAX_RECORD_SIZE];
        let bytes_written = encode_event(&event, &mut buf).unwrap();
        prop_assert!(bytes_written <= MAX_RECORD_SIZE);

        let mut decoded = FeedEvent::heartbeat(0, 0);
        let bytes_read = decode_event(&buf[..bytes_written], &mut decoded).unwrap();

        prop_assert_eq!(bytes_written, bytes_read);
        prop_assert_eq!(event, decoded);
    }

    #[test]
    fn prop_cents_prob_parsing(cents in 1u32..100u32) {
        let prob_from_cents = Prob::from_cents(cents).unwrap();
        let decimal_str = format!("0.{:02}", cents);
        let prob_from_decimal = parse_decimal_prob(&decimal_str).unwrap();

        prop_assert_eq!(prob_from_cents, prob_from_decimal);
        prop_assert_eq!(prob_from_cents.ppm(), cents * 10_000);
    }
}
