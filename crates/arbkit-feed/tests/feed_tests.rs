//! Comprehensive integration and end-to-end tests for `arbkit-feed`.

use arbkit_core::{Fee, Level, Prob};
use arbkit_feed::{
    kalshi_stake_fee, parse_decimal_prob, parse_decimal_size_cents, FeedError, FeedEvent,
    KalshiParser, PolymarketParser, TapeHeader, TapePlayer, TapeReader, TapeWriter, TradeSide,
    VENUE_KALSHI, VENUE_POLYMARKET,
};

#[test]
fn test_kalshi_full_lifecycle_and_sequence_tracking() {
    let mut parser = KalshiParser::new();

    // 1. Initial snapshot
    let snap_json = r#"{
        "type": "orderbook_snapshot",
        "seq": 1000,
        "msg": {
            "market_ticker": "KXNBAGAME-26AUG18BOSLAL",
            "yes": [[52, 100], [53, 250], [55, 500]],
            "no": [[47, 80], [45, 200]],
            "ts": 1700000000000
        }
    }"#;

    let msg = parser.parse_json(snap_json, 10, 100, 101, 0).unwrap();
    assert_eq!(msg.venue_id, VENUE_KALSHI);
    assert_eq!(msg.venue_seq, 1000);
    assert_eq!(msg.len(), 2);

    let yes_snap = &msg.events()[0];
    assert_eq!(yes_snap.venue_id(), VENUE_KALSHI);
    assert_eq!(yes_snap.market_id(), Some(10));
    assert_eq!(yes_snap.outcome_id(), Some(100));
    assert_eq!(yes_snap.seq(), Some(1000));
    assert_eq!(yes_snap.timestamp_micros(), 1_700_000_000_000_000);

    // 2. Contiguous delta update
    let delta_json = r#"{
        "type": "orderbook_delta",
        "seq": 1001,
        "msg": {
            "market_ticker": "KXNBAGAME-26AUG18BOSLAL",
            "price": 54,
            "delta": 300,
            "side": "yes",
            "ts": 1700000000100
        }
    }"#;

    let delta_msg = parser.parse_json(delta_json, 10, 100, 101, 0).unwrap();
    assert_eq!(delta_msg.len(), 1);
    match &delta_msg.events()[0] {
        FeedEvent::Delta {
            level,
            is_delete,
            seq,
            ..
        } => {
            assert_eq!(*seq, 1001);
            assert!(!is_delete);
            assert_eq!(level.price, Prob::from_cents(54).unwrap());
            assert_eq!(level.size, 300 * 54);
            assert_eq!(kalshi_stake_fee(level.price), Fee::StakeFeeBps(322));
        }
        _ => panic!("Expected Delta"),
    }

    // 3. Trade event
    let trade_json = r#"{
        "type": "trade",
        "seq": 1002,
        "msg": {
            "market_ticker": "KXNBAGAME-26AUG18BOSLAL",
            "yes_price": 54,
            "count": 50,
            "taker_side": "yes",
            "ts": 1700000000200
        }
    }"#;

    let trade_msg = parser.parse_json(trade_json, 10, 100, 101, 0).unwrap();
    assert_eq!(trade_msg.len(), 1);
    match &trade_msg.events()[0] {
        FeedEvent::Trade {
            price, size, side, ..
        } => {
            assert_eq!(*price, Prob::from_cents(54).unwrap());
            assert_eq!(*size, 50 * 54);
            assert_eq!(*side, TradeSide::Buy);
        }
        _ => panic!("Expected Trade"),
    }

    // 4. Sequence gap detection: jumped from 1002 to 1005
    let gap_json = r#"{
        "type": "orderbook_delta",
        "seq": 1005,
        "msg": {
            "market_ticker": "KXNBAGAME-26AUG18BOSLAL",
            "price": 54,
            "delta": 0,
            "side": "yes"
        }
    }"#;

    let err = parser.parse_json(gap_json, 10, 100, 101, 0).unwrap_err();
    match err {
        FeedError::SequenceGap {
            expected, received, ..
        } => {
            assert_eq!(expected, 1003);
            assert_eq!(received, 1005);
        }
        _ => panic!("Expected SequenceGap"),
    }
}

#[test]
fn test_polymarket_parsing_edge_cases() {
    let mut parser = PolymarketParser::new();

    // High-precision decimal prices
    assert_eq!(parse_decimal_prob("0.000001").unwrap().ppm(), 1);
    assert_eq!(parse_decimal_prob("0.999999").unwrap().ppm(), 999_999);
    assert_eq!(parse_decimal_prob("0.505050").unwrap().ppm(), 505_050);

    // Size parsing with fractions
    let p50 = Prob::from_cents(50).unwrap();
    assert_eq!(parse_decimal_size_cents("500.50", p50).unwrap(), 25_025);

    // Full snapshot parsing
    let book_json = r#"{
        "event_type": "book",
        "timestamp": "1700000000000",
        "seq": 500,
        "asks": [
            {"price": "0.52", "size": "1000"},
            {"price": "0.53", "size": "1500"},
            {"price": "0.54", "size": "2000"}
        ],
        "bids": [
            {"price": "0.48", "size": "800"},
            {"price": "0.47", "size": "1200"}
        ]
    }"#;

    let msg = parser.parse_json(book_json, 20, 200, 0).unwrap();
    assert_eq!(msg.len(), 1);
    match &msg.events()[0] {
        FeedEvent::Snapshot {
            num_levels, levels, ..
        } => {
            assert_eq!(*num_levels, 5); // 3 asks + 2 bids
            assert_eq!(levels[0].price.ppm(), 520_000);
            assert_eq!(levels[0].size, 52_000);
            assert_eq!(levels[1].price.ppm(), 530_000);
            assert_eq!(levels[1].size, 79_500);
        }
        _ => panic!("Expected Snapshot"),
    }
}

#[test]
fn test_tape_recording_and_zero_allocation_batch_replay() {
    let mut tape_buffer = Vec::new();
    let sample_events = [
        FeedEvent::snapshot(
            VENUE_KALSHI,
            1,
            10,
            100,
            1_000_000,
            &[
                Level {
                    price: Prob::from_cents(48).unwrap(),
                    size: 50_000,
                },
                Level {
                    price: Prob::from_cents(52).unwrap(),
                    size: 100_000,
                },
            ],
        ),
        FeedEvent::delta(
            VENUE_KALSHI,
            1,
            10,
            101,
            1_000_100,
            Level {
                price: Prob::from_cents(49).unwrap(),
                size: 20_000,
            },
            false,
        ),
        FeedEvent::trade(
            VENUE_KALSHI,
            1,
            10,
            102,
            1_000_200,
            Prob::from_cents(49).unwrap(),
            10_000,
            TradeSide::Buy,
        ),
        FeedEvent::heartbeat(VENUE_KALSHI, 1_000_300),
        FeedEvent::snapshot(
            VENUE_POLYMARKET,
            1,
            11,
            200,
            1_000_400,
            &[Level {
                price: Prob::from_cents(51).unwrap(),
                size: 75_000,
            }],
        ),
        FeedEvent::halt(VENUE_POLYMARKET, 1, Some(11), 1_000_500, 1),
    ];

    // 1. Record events
    {
        let header = TapeHeader::new(1_000_000);
        let mut writer = TapeWriter::with_header(&mut tape_buffer, header).unwrap();
        writer.write_batch(&sample_events).unwrap();
        writer.flush().unwrap();
        assert_eq!(writer.events_written(), 6);
    }

    // 2. Read in batch into a fixed preallocated array (zero allocations)
    {
        let mut reader = TapeReader::new(tape_buffer.as_slice()).unwrap();
        let mut batch_buffer = [FeedEvent::heartbeat(0, 0); 16];
        let read_count = reader.read_batch(&mut batch_buffer).unwrap();

        assert_eq!(read_count, 6);
        for (i, expected) in sample_events.iter().enumerate() {
            assert_eq!(&batch_buffer[i], expected);
        }
    }

    // 3. Player with filters
    {
        let reader = TapeReader::new(tape_buffer.as_slice()).unwrap();
        let mut player = TapePlayer::new(reader).with_venue_filter(VENUE_POLYMARKET);

        let mut poly_events = [FeedEvent::heartbeat(0, 0); 8];
        let count = player.play_into(&mut poly_events).unwrap();
        assert_eq!(count, 2);
        assert_eq!(player.events_played(), 2);
        assert_eq!(poly_events[0].venue_id(), VENUE_POLYMARKET);
        assert!(matches!(poly_events[0], FeedEvent::Snapshot { .. }));
        assert!(matches!(poly_events[1], FeedEvent::Halt { .. }));
    }
}

#[test]
fn test_tape_error_handling() {
    // 1. Truncated header
    let short_data = [0u8; 10];
    assert!(TapeReader::new(short_data.as_slice()).is_err());

    // 2. Bad magic bytes
    let mut bad_magic = [0u8; 64];
    bad_magic[0..8].copy_from_slice(b"BADMAGIC");
    assert!(TapeReader::new(bad_magic.as_slice()).is_err());

    // 3. Truncated payload
    let mut valid_header = TapeHeader::default().encode().to_vec();
    valid_header.push(1); // snapshot record tag without payload
    let mut reader = TapeReader::new(valid_header.as_slice()).unwrap();
    let mut event = FeedEvent::heartbeat(0, 0);
    assert!(reader.read_event(&mut event).is_err());
}
