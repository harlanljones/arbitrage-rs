//! Kalshi WebSocket market data parser and sequence tracker.
//!
//! Normalizes Kalshi JSON message frames (snapshots, deltas, trades, heartbeats, halts)
//! into zero-allocation [`FeedEvent`] and [`FeedMessage`] records. Prices in whole cents
//! (`1..=99`) are mapped directly to canonical [`Prob`] values.

use super::VENUE_KALSHI;
use crate::error::{FeedError, Result};
use crate::event::{FeedEvent, FeedMessage, TradeSide};
use arbkit_core::fee::kalshi_stake_fee_bps;
use arbkit_core::{Cents, Fee, Level, MarketId, OutcomeId, Prob, MAX_LEVELS};
use serde::Deserialize;

/// Sequence tracker for monitoring contiguous Kalshi feed sequence numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KalshiSequenceTracker {
    last_seq: u64,
    initialized: bool,
}

impl KalshiSequenceTracker {
    /// Creates a new uninitialized sequence tracker.
    pub const fn new() -> Self {
        Self {
            last_seq: 0,
            initialized: false,
        }
    }

    /// Resets the sequence tracker to a known sequence number (e.g. upon receiving a snapshot).
    pub fn reset_to(&mut self, seq: u64) {
        self.last_seq = seq;
        self.initialized = true;
    }

    /// Validates and advances to the next expected sequence number.
    ///
    /// Returns `Ok(())` if the sequence is unbroken, or `Err(FeedError::SequenceGap)`
    /// if a gap is detected.
    pub fn accept_seq(&mut self, seq: u64) -> Result<()> {
        if !self.initialized {
            self.last_seq = seq;
            self.initialized = true;
            return Ok(());
        }

        let expected = self.last_seq + 1;
        if seq != expected {
            return Err(FeedError::SequenceGap {
                venue_id: VENUE_KALSHI,
                expected,
                received: seq,
            });
        }

        self.last_seq = seq;
        Ok(())
    }

    /// Returns the last accepted sequence number.
    #[inline]
    pub const fn last_seq(&self) -> u64 {
        self.last_seq
    }

    /// Returns `true` if this tracker has accepted at least one sequence number.
    #[inline]
    pub const fn is_initialized(&self) -> bool {
        self.initialized
    }
}

/// Helper function to compute the Kalshi stake fee model for a given price.
#[inline]
pub fn kalshi_stake_fee(price: Prob) -> Fee {
    Fee::StakeFeeBps(kalshi_stake_fee_bps(price))
}

/// Kalshi market data message parser.
#[derive(Debug, Default)]
pub struct KalshiParser {
    seq_tracker: KalshiSequenceTracker,
}

impl KalshiParser {
    /// Creates a new [`KalshiParser`] with default sequence tracking.
    pub fn new() -> Self {
        Self::default()
    }

    /// Accesses the underlying [`KalshiSequenceTracker`].
    pub fn sequence_tracker(&self) -> &KalshiSequenceTracker {
        &self.seq_tracker
    }

    /// Accesses the mutable underlying [`KalshiSequenceTracker`].
    pub fn sequence_tracker_mut(&mut self) -> &mut KalshiSequenceTracker {
        &mut self.seq_tracker
    }

    /// Parses a raw Kalshi JSON text frame into a [`FeedMessage`].
    ///
    /// # Arguments
    /// * `raw_json` - JSON string received from the WebSocket.
    /// * `market_id` - Canonical market identifier.
    /// * `yes_outcome_id` - Canonical outcome ID for the "yes" contract.
    /// * `no_outcome_id` - Canonical outcome ID for the "no" contract.
    /// * `default_timestamp_ns` - Fallback ingress timestamp if omitted in payload.
    pub fn parse_json(
        &mut self,
        raw_json: &str,
        market_id: MarketId,
        yes_outcome_id: OutcomeId,
        no_outcome_id: OutcomeId,
        default_timestamp_ns: u64,
    ) -> Result<FeedMessage> {
        let envelope: KalshiEnvelope = serde_json::from_str(raw_json)
            .map_err(|e| FeedError::ParseError(format!("Kalshi JSON error: {e}")))?;

        let seq = envelope.seq.unwrap_or(0);
        let msg_type = envelope.msg_type.as_deref().unwrap_or("");

        let timestamp_ns = envelope
            .msg
            .as_ref()
            .and_then(|m| m.ts)
            .map(|ts_ms| ts_ms * 1_000_000)
            .unwrap_or(default_timestamp_ns);

        let mut feed_msg = FeedMessage::new(VENUE_KALSHI, timestamp_ns, seq);

        match msg_type {
            "orderbook_snapshot" | "snapshot" => {
                if seq > 0 {
                    self.seq_tracker.reset_to(seq);
                }
                if let Some(msg) = envelope.msg {
                    // Parse "yes" levels
                    if let Some(yes_levels) = msg.yes {
                        let (parsed, count) = parse_kalshi_levels(&yes_levels)?;
                        feed_msg.push(FeedEvent::snapshot(
                            VENUE_KALSHI,
                            market_id,
                            yes_outcome_id,
                            seq,
                            timestamp_ns,
                            &parsed[..count],
                        ));
                    }
                    // Parse "no" levels
                    if let Some(no_levels) = msg.no {
                        let (parsed, count) = parse_kalshi_levels(&no_levels)?;
                        feed_msg.push(FeedEvent::snapshot(
                            VENUE_KALSHI,
                            market_id,
                            no_outcome_id,
                            seq,
                            timestamp_ns,
                            &parsed[..count],
                        ));
                    }
                }
            }

            "orderbook_delta" | "delta" => {
                if seq > 0 {
                    self.seq_tracker.accept_seq(seq)?;
                }
                if let Some(msg) = envelope.msg {
                    let cents = msg.price.ok_or(FeedError::MissingField("price"))?;
                    let prob = Prob::from_cents(cents)
                        .map_err(|e| FeedError::InvalidPrice(format!("{e}")))?;
                    let delta_count = msg.delta.unwrap_or(0);
                    let side = msg.side.as_deref().unwrap_or("yes");
                    let outcome_id = if side.eq_ignore_ascii_case("no") {
                        no_outcome_id
                    } else {
                        yes_outcome_id
                    };

                    let is_delete = delta_count <= 0;
                    let size_cents: Cents = if is_delete {
                        0
                    } else {
                        delta_count as Cents * cents as Cents
                    };

                    feed_msg.push(FeedEvent::delta(
                        VENUE_KALSHI,
                        market_id,
                        outcome_id,
                        seq,
                        timestamp_ns,
                        Level {
                            price: prob,
                            size: size_cents,
                        },
                        is_delete,
                    ));
                }
            }

            "trade" | "fill" => {
                if seq > 0 {
                    let _ = self.seq_tracker.accept_seq(seq);
                }
                if let Some(msg) = envelope.msg {
                    let cents = msg
                        .yes_price
                        .or(msg.price)
                        .ok_or(FeedError::MissingField("price"))?;
                    let prob = Prob::from_cents(cents)
                        .map_err(|e| FeedError::InvalidPrice(format!("{e}")))?;
                    let count = msg.count.or(msg.delta).unwrap_or(1);
                    let size_cents = count as Cents * cents as Cents;
                    let side_str = msg.taker_side.as_deref().or(msg.side.as_deref());
                    let side = match side_str {
                        Some("yes") | Some("buy") => TradeSide::Buy,
                        Some("no") | Some("sell") => TradeSide::Sell,
                        _ => TradeSide::Unknown,
                    };

                    feed_msg.push(FeedEvent::trade(
                        VENUE_KALSHI,
                        market_id,
                        yes_outcome_id,
                        seq,
                        timestamp_ns,
                        prob,
                        size_cents,
                        side,
                    ));
                }
            }

            "heartbeat" | "ping" | "pong" => {
                feed_msg.push(FeedEvent::heartbeat(VENUE_KALSHI, timestamp_ns));
            }

            "market_status" | "halt" => {
                let reason_code = envelope
                    .msg
                    .and_then(|m| m.status)
                    .map(|s| match s.as_str() {
                        "halted" | "suspended" => 1,
                        "closed" | "settled" => 2,
                        _ => 0,
                    })
                    .unwrap_or(0);

                feed_msg.push(FeedEvent::halt(
                    VENUE_KALSHI,
                    market_id,
                    None,
                    timestamp_ns,
                    reason_code,
                ));
            }

            _ => {
                // If unrecognized type but payload contains heartbeat or generic fields
                if envelope.msg.is_none() && seq == 0 {
                    feed_msg.push(FeedEvent::heartbeat(VENUE_KALSHI, timestamp_ns));
                }
            }
        }

        Ok(feed_msg)
    }
}

/// Helper function to parse Kalshi `[[cents, count], ...]` arrays into `[Level; MAX_LEVELS]` and count.
fn parse_kalshi_levels(raw_levels: &[[i64; 2]]) -> Result<([Level; MAX_LEVELS], usize)> {
    let mut levels = [Level {
        price: Prob::CERTAIN,
        size: 0,
    }; MAX_LEVELS];

    let count = raw_levels.len().min(MAX_LEVELS);
    for (i, item) in raw_levels.iter().take(count).enumerate() {
        let cents = item[0] as u32;
        let quantity = item[1];
        let prob = Prob::from_cents(cents).map_err(|e| FeedError::InvalidPrice(format!("{e}")))?;
        let size_cents = quantity * cents as i64;
        levels[i] = Level {
            price: prob,
            size: size_cents,
        };
    }

    Ok((levels, count))
}

#[derive(Debug, Deserialize)]
struct KalshiEnvelope {
    #[serde(rename = "type")]
    msg_type: Option<String>,
    seq: Option<u64>,
    msg: Option<KalshiMsg>,
}

#[derive(Debug, Deserialize)]
struct KalshiMsg {
    ts: Option<u64>,
    price: Option<u32>,
    yes_price: Option<u32>,
    delta: Option<i64>,
    count: Option<i64>,
    side: Option<String>,
    taker_side: Option<String>,
    status: Option<String>,
    yes: Option<Vec<[i64; 2]>>,
    no: Option<Vec<[i64; 2]>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kalshi_sequence_tracking() {
        let mut tracker = KalshiSequenceTracker::new();
        assert!(!tracker.is_initialized());

        assert!(tracker.accept_seq(10).is_ok());
        assert!(tracker.is_initialized());
        assert_eq!(tracker.last_seq(), 10);

        assert!(tracker.accept_seq(11).is_ok());
        assert_eq!(tracker.last_seq(), 11);

        // Gap from 11 to 13 must error
        let gap_err = tracker.accept_seq(13).unwrap_err();
        match gap_err {
            FeedError::SequenceGap {
                expected, received, ..
            } => {
                assert_eq!(expected, 12);
                assert_eq!(received, 13);
            }
            _ => panic!("Expected SequenceGap error"),
        }

        // Reset clears gap
        tracker.reset_to(50);
        assert_eq!(tracker.last_seq(), 50);
        assert!(tracker.accept_seq(51).is_ok());
    }

    #[test]
    fn test_parse_kalshi_snapshot() {
        let mut parser = KalshiParser::new();
        let json = r#"{
            "type": "orderbook_snapshot",
            "seq": 100,
            "msg": {
                "market_ticker": "KXNBAGAME-TEST",
                "yes": [[52, 100], [55, 200]],
                "no": [[45, 150]],
                "ts": 1700000000000
            }
        }"#;

        let msg = parser.parse_json(json, 1, 10, 11, 0).unwrap();
        assert_eq!(msg.venue_id, VENUE_KALSHI);
        assert_eq!(msg.venue_seq, 100);
        assert_eq!(msg.len(), 2);

        let yes_snap = &msg.events()[0];
        match yes_snap {
            FeedEvent::Snapshot {
                venue_id,
                market_id,
                outcome_id,
                seq,
                levels,
                num_levels,
                ..
            } => {
                assert_eq!(*venue_id, VENUE_KALSHI);
                assert_eq!(*market_id, 1);
                assert_eq!(*outcome_id, 10);
                assert_eq!(*seq, 100);
                assert_eq!(*num_levels, 2);
                assert_eq!(levels[0].price, Prob::from_cents(52).unwrap());
                assert_eq!(levels[0].size, 5200); // 100 contracts * 52 cents
                assert_eq!(levels[1].price, Prob::from_cents(55).unwrap());
                assert_eq!(levels[1].size, 11000); // 200 contracts * 55 cents
            }
            _ => panic!("Expected Snapshot event"),
        }

        let no_snap = &msg.events()[1];
        match no_snap {
            FeedEvent::Snapshot {
                outcome_id,
                num_levels,
                levels,
                ..
            } => {
                assert_eq!(*outcome_id, 11);
                assert_eq!(*num_levels, 1);
                assert_eq!(levels[0].price, Prob::from_cents(45).unwrap());
                assert_eq!(levels[0].size, 6750); // 150 contracts * 45 cents
            }
            _ => panic!("Expected Snapshot event"),
        }
    }

    #[test]
    fn test_parse_kalshi_delta_and_fees() {
        let mut parser = KalshiParser::new();
        // Initialize seq
        parser.sequence_tracker_mut().reset_to(100);

        let json = r#"{
            "type": "orderbook_delta",
            "seq": 101,
            "msg": {
                "market_ticker": "KXNBAGAME-TEST",
                "price": 50,
                "delta": 25,
                "side": "yes",
                "ts": 1700000001000
            }
        }"#;

        let msg = parser.parse_json(json, 1, 10, 11, 0).unwrap();
        assert_eq!(msg.len(), 1);
        let event = &msg.events()[0];
        match event {
            FeedEvent::Delta {
                level,
                is_delete,
                outcome_id,
                ..
            } => {
                assert_eq!(*outcome_id, 10);
                assert!(!is_delete);
                assert_eq!(level.price, Prob::from_cents(50).unwrap());
                assert_eq!(level.size, 1250); // 25 * 50 cents

                // Verify fee
                let fee = kalshi_stake_fee(level.price);
                assert_eq!(fee, Fee::StakeFeeBps(350));
            }
            _ => panic!("Expected Delta event"),
        }
    }

    #[test]
    fn test_parse_kalshi_trade_and_heartbeat() {
        let mut parser = KalshiParser::new();
        let trade_json = r#"{
            "type": "trade",
            "seq": 1,
            "msg": {
                "market_ticker": "KXNBAGAME-TEST",
                "yes_price": 48,
                "count": 50,
                "taker_side": "yes",
                "ts": 1700000002000
            }
        }"#;

        let msg = parser.parse_json(trade_json, 1, 10, 11, 0).unwrap();
        assert_eq!(msg.len(), 1);
        match &msg.events()[0] {
            FeedEvent::Trade {
                price, size, side, ..
            } => {
                assert_eq!(*price, Prob::from_cents(48).unwrap());
                assert_eq!(*size, 2400); // 50 * 48
                assert_eq!(*side, TradeSide::Buy);
            }
            _ => panic!("Expected Trade event"),
        }

        let hb_json = r#"{"type": "heartbeat", "msg": {"ts": 1700000003000}}"#;
        let hb_msg = parser.parse_json(hb_json, 1, 10, 11, 0).unwrap();
        assert_eq!(hb_msg.len(), 1);
        assert!(matches!(hb_msg.events()[0], FeedEvent::Heartbeat { .. }));
    }
}
