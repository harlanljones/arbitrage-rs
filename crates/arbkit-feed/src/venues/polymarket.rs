//! Polymarket CLOB WebSocket market channel message parser.
//!
//! Normalizes Polymarket CLOB market channel payloads into zero-allocation
//! [`FeedEvent`] and [`FeedMessage`] records. Parses decimal string quotes
//! (e.g. `"0.52"`) directly into canonical [`Prob`] values using fixed-point
//! arithmetic to eliminate floating point rounding error at the boundary.

use super::VENUE_POLYMARKET;
use crate::error::{FeedError, Result};
use crate::event::{FeedEvent, FeedMessage, TradeSide};
use arbkit_core::{Cents, Level, MarketId, OutcomeId, Prob, MAX_LEVELS, PPM};
use serde::Deserialize;

/// Parses a decimal string price (e.g. `"0.52"`, `"0.505"`, `"1.0"`) directly into [`Prob`].
///
/// Implements fixed-point base-10 arithmetic without intermediate floating-point
/// representation to prevent reciprocal chain drift.
pub fn parse_decimal_prob(s: &str) -> Result<Prob> {
    let s = s.trim();
    if s.is_empty() {
        return Err(FeedError::InvalidPrice("empty price string".to_string()));
    }

    let (integer_part, frac_part) = match s.split_once('.') {
        Some((int_s, frac_s)) => (int_s, frac_s),
        None => (s, ""),
    };

    let int_val: u64 = integer_part
        .parse()
        .map_err(|_| FeedError::InvalidPrice(format!("invalid integer part: '{integer_part}'")))?;

    let mut ppm_val = int_val
        .checked_mul(PPM as u64)
        .ok_or_else(|| FeedError::InvalidPrice("integer price overflow".to_string()))?;

    if !frac_part.is_empty() {
        let mut frac_ppm = 0u64;
        let mut divisor = 1u64;
        let mut digits_counted = 0;

        for ch in frac_part.chars() {
            if !ch.is_ascii_digit() {
                return Err(FeedError::InvalidPrice(format!(
                    "invalid character in price: '{ch}'"
                )));
            }
            let digit = ch as u64 - '0' as u64;
            if digits_counted < 6 {
                frac_ppm = frac_ppm * 10 + digit;
                divisor *= 10;
                digits_counted += 1;
            } else if digits_counted == 6 {
                // Round to nearest using 7th digit
                if digit >= 5 {
                    frac_ppm += 1;
                }
                digits_counted += 1;
            }
        }

        // Scale up to 6 decimal places (PPM)
        while divisor < PPM as u64 {
            frac_ppm *= 10;
            divisor *= 10;
        }

        ppm_val += frac_ppm;
    }

    if ppm_val == 0 || ppm_val > PPM as u64 {
        return Err(FeedError::InvalidPrice(format!(
            "price {ppm_val} ppm is outside 1..=1_000_000"
        )));
    }

    Prob::from_ppm(ppm_val as u32).map_err(|e| FeedError::InvalidPrice(format!("{e}")))
}

/// Parses a decimal size string (in token / contract units) into stake in [`Cents`].
///
/// Multiplies the contract count by the price in implied probability to compute
/// the total stake capacity absorbable by the level.
pub fn parse_decimal_size_cents(size_str: &str, price: Prob) -> Result<Cents> {
    let s = size_str.trim();
    if s.is_empty() {
        return Err(FeedError::ParseError("empty size string".to_string()));
    }

    let (int_s, frac_s) = match s.split_once('.') {
        Some((i, f)) => (i, f),
        None => (s, ""),
    };

    let int_part: u64 = int_s
        .parse()
        .map_err(|_| FeedError::ParseError(format!("invalid size: '{size_str}'")))?;

    // Parse fraction up to 2 decimal places (hundredths of contract)
    let frac_cents = if !frac_s.is_empty() {
        let mut frac = 0u64;
        let mut div = 1u64;
        for ch in frac_s.chars().take(2) {
            if ch.is_ascii_digit() {
                frac = frac * 10 + (ch as u64 - '0' as u64);
                div *= 10;
            }
        }
        while div < 100 {
            frac *= 10;
            div *= 10;
        }
        frac
    } else {
        0
    };

    // Total contract count in hundredths (e.g. 100 contracts = 10_000 hundredths)
    let hundredths_contracts = int_part * 100 + frac_cents;

    // Stake in cents = contracts * price_prob (price in ppm / 1_000_000)
    // = (hundredths_contracts / 100) * (ppm / 10_000) cents
    // = (hundredths_contracts * ppm) / 1_000_000
    let stake_cents = (hundredths_contracts as u128 * price.ppm() as u128) / (PPM as u128);
    Ok(stake_cents as Cents)
}

/// Polymarket CLOB market channel parser.
#[derive(Debug, Default)]
pub struct PolymarketParser;

impl PolymarketParser {
    /// Creates a new [`PolymarketParser`].
    pub fn new() -> Self {
        Self
    }

    /// Parses a Polymarket CLOB WebSocket market channel JSON frame.
    ///
    /// # Arguments
    /// * `raw_json` - JSON string from WebSocket frame.
    /// * `market_id` - Interned market identifier.
    /// * `outcome_id` - Interned outcome identifier.
    /// * `default_timestamp_ns` - Fallback timestamp if not present in message.
    pub fn parse_json(
        &mut self,
        raw_json: &str,
        market_id: MarketId,
        outcome_id: OutcomeId,
        default_timestamp_ns: u64,
    ) -> Result<FeedMessage> {
        let frame: PolymarketFrame = serde_json::from_str(raw_json)
            .map_err(|e| FeedError::ParseError(format!("Polymarket JSON error: {e}")))?;

        let event_type = frame
            .event_type
            .as_deref()
            .or(frame.event.as_deref())
            .unwrap_or("");

        let timestamp_ns = frame
            .timestamp
            .as_ref()
            .and_then(|ts_str| ts_str.parse::<u64>().ok())
            .map(|ts_ms| ts_ms * 1_000_000)
            .unwrap_or(default_timestamp_ns);

        let seq = frame.seq.unwrap_or(0);
        let mut feed_msg = FeedMessage::new(VENUE_POLYMARKET, timestamp_ns, seq);

        match event_type {
            "book" | "snapshot" => {
                // Parse order book snapshot
                let mut levels = [Level {
                    price: Prob::CERTAIN,
                    size: 0,
                }; MAX_LEVELS];
                let mut count = 0usize;

                // Priority: asks (offers to buy from, longest price first)
                if let Some(asks) = frame.asks {
                    for ask in asks {
                        if count >= MAX_LEVELS {
                            break;
                        }
                        if let (Some(price_s), Some(size_s)) = (ask.price, ask.size) {
                            let prob = parse_decimal_prob(&price_s)?;
                            let size = parse_decimal_size_cents(&size_s, prob)?;
                            if size > 0 {
                                levels[count] = Level { price: prob, size };
                                count += 1;
                            }
                        }
                    }
                }

                // If asks are fewer than MAX_LEVELS, also populate bids
                if count < MAX_LEVELS {
                    if let Some(bids) = frame.bids {
                        for bid in bids {
                            if count >= MAX_LEVELS {
                                break;
                            }
                            if let (Some(price_s), Some(size_s)) = (bid.price, bid.size) {
                                if let Ok(prob) = parse_decimal_prob(&price_s) {
                                    if let Ok(size) = parse_decimal_size_cents(&size_s, prob) {
                                        if size > 0 {
                                            levels[count] = Level { price: prob, size };
                                            count += 1;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                feed_msg.push(FeedEvent::snapshot(
                    VENUE_POLYMARKET,
                    market_id,
                    outcome_id,
                    seq,
                    timestamp_ns,
                    &levels[..count],
                ));
            }

            "price_change" | "delta" => {
                if let Some(changes) = frame.changes {
                    for change in changes {
                        if let (Some(price_s), Some(size_s)) = (change.price, change.size) {
                            let prob = parse_decimal_prob(&price_s)?;
                            let size = parse_decimal_size_cents(&size_s, prob).unwrap_or(0);
                            let is_delete = size == 0;

                            feed_msg.push(FeedEvent::delta(
                                VENUE_POLYMARKET,
                                market_id,
                                outcome_id,
                                seq,
                                timestamp_ns,
                                Level { price: prob, size },
                                is_delete,
                            ));
                        }
                    }
                }
            }

            "trade" | "last_trade_price" => {
                if let (Some(price_s), Some(size_s)) = (frame.price, frame.size) {
                    let prob = parse_decimal_prob(&price_s)?;
                    let size = parse_decimal_size_cents(&size_s, prob)?;
                    let side = match frame.side.as_deref() {
                        Some("BUY") | Some("buy") => TradeSide::Buy,
                        Some("SELL") | Some("sell") => TradeSide::Sell,
                        _ => TradeSide::Unknown,
                    };

                    feed_msg.push(FeedEvent::trade(
                        VENUE_POLYMARKET,
                        market_id,
                        outcome_id,
                        seq,
                        timestamp_ns,
                        prob,
                        size,
                        side,
                    ));
                }
            }

            "heartbeat" | "ping" => {
                feed_msg.push(FeedEvent::heartbeat(VENUE_POLYMARKET, timestamp_ns));
            }

            "market_resolved" | "halt" => {
                feed_msg.push(FeedEvent::halt(
                    VENUE_POLYMARKET,
                    market_id,
                    Some(outcome_id),
                    timestamp_ns,
                    1,
                ));
            }

            _ => {
                if frame.asks.is_none() && frame.changes.is_none() && frame.price.is_none() {
                    feed_msg.push(FeedEvent::heartbeat(VENUE_POLYMARKET, timestamp_ns));
                }
            }
        }

        Ok(feed_msg)
    }
}

#[derive(Debug, Deserialize)]
struct PolymarketFrame {
    event_type: Option<String>,
    event: Option<String>,
    timestamp: Option<String>,
    seq: Option<u64>,
    price: Option<String>,
    size: Option<String>,
    side: Option<String>,
    bids: Option<Vec<PolymarketPriceLevel>>,
    asks: Option<Vec<PolymarketPriceLevel>>,
    changes: Option<Vec<PolymarketChange>>,
}

#[derive(Debug, Deserialize)]
struct PolymarketPriceLevel {
    price: Option<String>,
    size: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PolymarketChange {
    price: Option<String>,
    size: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_decimal_prob() {
        assert_eq!(parse_decimal_prob("0.52").unwrap().ppm(), 520_000);
        assert_eq!(parse_decimal_prob("0.5").unwrap().ppm(), 500_000);
        assert_eq!(parse_decimal_prob("0.05").unwrap().ppm(), 50_000);
        assert_eq!(parse_decimal_prob("0.005").unwrap().ppm(), 5_000);
        assert_eq!(parse_decimal_prob("0.0005").unwrap().ppm(), 500);
        assert_eq!(parse_decimal_prob("0.00005").unwrap().ppm(), 50);
        assert_eq!(parse_decimal_prob("0.000005").unwrap().ppm(), 5);
        assert_eq!(parse_decimal_prob("0.000001").unwrap().ppm(), 1);
        assert_eq!(parse_decimal_prob("1.0").unwrap().ppm(), 1_000_000);
        assert_eq!(parse_decimal_prob("1").unwrap().ppm(), 1_000_000);
        assert_eq!(parse_decimal_prob("0.505").unwrap().ppm(), 505_000);

        // Invalid cases
        assert!(parse_decimal_prob("0").is_err());
        assert!(parse_decimal_prob("0.000000").is_err());
        assert!(parse_decimal_prob("1.5").is_err());
        assert!(parse_decimal_prob("abc").is_err());
    }

    #[test]
    fn test_parse_decimal_size_cents() {
        let prob = Prob::from_cents(50).unwrap(); // 500_000 ppm
                                                  // 100 contracts at 50c = $50 = 5000 cents stake
        let cents = parse_decimal_size_cents("100", prob).unwrap();
        assert_eq!(cents, 5000);

        // 10 contracts at 52c = $5.20 = 520 cents stake
        let prob_52 = Prob::from_cents(52).unwrap();
        let cents_52 = parse_decimal_size_cents("10", prob_52).unwrap();
        assert_eq!(cents_52, 520);
    }

    #[test]
    fn test_parse_polymarket_book_snapshot() {
        let mut parser = PolymarketParser::new();
        let json = r#"{
            "event_type": "book",
            "timestamp": "1700000000000",
            "asks": [
                {"price": "0.52", "size": "1000"},
                {"price": "0.54", "size": "500"}
            ],
            "bids": [
                {"price": "0.50", "size": "800"}
            ]
        }"#;

        let msg = parser.parse_json(json, 2, 20, 0).unwrap();
        assert_eq!(msg.venue_id, VENUE_POLYMARKET);
        assert_eq!(msg.len(), 1);

        match &msg.events()[0] {
            FeedEvent::Snapshot {
                venue_id,
                market_id,
                outcome_id,
                num_levels,
                levels,
                ..
            } => {
                assert_eq!(*venue_id, VENUE_POLYMARKET);
                assert_eq!(*market_id, 2);
                assert_eq!(*outcome_id, 20);
                assert_eq!(*num_levels, 3); // 2 asks + 1 bid
                assert_eq!(levels[0].price.ppm(), 520_000);
                assert_eq!(levels[0].size, 52_000); // 1000 contracts * 52c = $520 = 52000 cents
            }
            _ => panic!("Expected Snapshot event"),
        }
    }

    #[test]
    fn test_parse_polymarket_price_change() {
        let mut parser = PolymarketParser::new();
        let json = r#"{
            "event_type": "price_change",
            "timestamp": "1700000001000",
            "changes": [
                {"price": "0.53", "size": "200"},
                {"price": "0.54", "size": "0"}
            ]
        }"#;

        let msg = parser.parse_json(json, 2, 20, 0).unwrap();
        assert_eq!(msg.len(), 2);

        match &msg.events()[0] {
            FeedEvent::Delta {
                level, is_delete, ..
            } => {
                assert_eq!(level.price.ppm(), 530_000);
                assert_eq!(level.size, 10_600); // 200 * 53c
                assert!(!is_delete);
            }
            _ => panic!("Expected Delta event"),
        }

        match &msg.events()[1] {
            FeedEvent::Delta { is_delete, .. } => {
                assert!(is_delete);
            }
            _ => panic!("Expected Delta event"),
        }
    }

    #[test]
    fn test_parse_polymarket_trade() {
        let mut parser = PolymarketParser::new();
        let json = r#"{
            "event_type": "trade",
            "timestamp": "1700000002000",
            "price": "0.55",
            "size": "400",
            "side": "BUY"
        }"#;

        let msg = parser.parse_json(json, 2, 20, 0).unwrap();
        assert_eq!(msg.len(), 1);
        match &msg.events()[0] {
            FeedEvent::Trade {
                price, size, side, ..
            } => {
                assert_eq!(price.ppm(), 550_000);
                assert_eq!(*size, 22_000); // 400 * 55c
                assert_eq!(*side, TradeSide::Buy);
            }
            _ => panic!("Expected Trade event"),
        }
    }
}
