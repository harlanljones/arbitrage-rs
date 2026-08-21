//! Binary tape format specification, header serialization, and zero-allocation record codec.

use crate::error::{FeedError, Result};
use crate::event::{FeedEvent, TradeSide};
use arbkit_core::{Level, Prob, MAX_LEVELS};

/// Magic bytes identifying an arbkit market tape file.
pub const TAPE_MAGIC: [u8; 8] = *b"ARBTAPE\x01";

/// Current tape format specification version.
pub const TAPE_VERSION: u16 = 1;

/// Fixed size of the binary tape file header in bytes.
pub const TAPE_HEADER_SIZE: usize = 64;

/// Maximum possible serialized record size in bytes.
pub const MAX_RECORD_SIZE: usize = 128;

/// Binary record tag for an order book snapshot event.
pub const RECORD_TAG_SNAPSHOT: u8 = 1;
/// Binary record tag for an order book delta update event.
pub const RECORD_TAG_DELTA: u8 = 2;
/// Binary record tag for a trade execution event.
pub const RECORD_TAG_TRADE: u8 = 3;
/// Binary record tag for a heartbeat event.
pub const RECORD_TAG_HEARTBEAT: u8 = 4;
/// Binary record tag for a market halt event.
pub const RECORD_TAG_HALT: u8 = 5;

/// Header metadata for a binary market tape file or buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TapeHeader {
    /// Magic identifier bytes (`ARBTAPE\x01`).
    pub magic: [u8; 8],
    /// Binary format version.
    pub version: u16,
    /// Format flags (reserved for compression/checksumming).
    pub flags: u16,
    /// Unix timestamp in nanoseconds when this tape was created.
    pub created_timestamp_ns: u64,
    /// Total number of events contained in this tape (0 if streaming/unknown).
    pub event_count: u64,
    /// Bitmask of interned venue IDs present on the tape.
    pub venue_mask: u32,
    /// Reserved zero padding.
    pub reserved: [u8; 32],
}

impl Default for TapeHeader {
    fn default() -> Self {
        Self {
            magic: TAPE_MAGIC,
            version: TAPE_VERSION,
            flags: 0,
            created_timestamp_ns: 0,
            event_count: 0,
            venue_mask: 0,
            reserved: [0u8; 32],
        }
    }
}

impl TapeHeader {
    /// Creates a new [`TapeHeader`] with current version and given timestamp.
    pub fn new(created_timestamp_ns: u64) -> Self {
        Self {
            created_timestamp_ns,
            ..Default::default()
        }
    }

    /// Serializes the header into a 64-byte array.
    pub fn encode(&self) -> [u8; TAPE_HEADER_SIZE] {
        let mut buf = [0u8; TAPE_HEADER_SIZE];
        buf[0..8].copy_from_slice(&self.magic);
        buf[8..10].copy_from_slice(&self.version.to_le_bytes());
        buf[10..12].copy_from_slice(&self.flags.to_le_bytes());
        buf[12..20].copy_from_slice(&self.created_timestamp_ns.to_le_bytes());
        buf[20..28].copy_from_slice(&self.event_count.to_le_bytes());
        buf[28..32].copy_from_slice(&self.venue_mask.to_le_bytes());
        buf[32..64].copy_from_slice(&self.reserved);
        buf
    }

    /// Deserializes and validates a [`TapeHeader`] from a 64-byte slice.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < TAPE_HEADER_SIZE {
            return Err(FeedError::InvalidTapeHeader("header slice too short"));
        }

        let mut magic = [0u8; 8];
        magic.copy_from_slice(&bytes[0..8]);
        if magic != TAPE_MAGIC {
            return Err(FeedError::InvalidTapeHeader("magic bytes mismatch"));
        }

        let version = u16::from_le_bytes([bytes[8], bytes[9]]);
        if version != TAPE_VERSION {
            return Err(FeedError::UnsupportedTapeVersion(version, TAPE_VERSION));
        }

        let flags = u16::from_le_bytes([bytes[10], bytes[11]]);
        let created_timestamp_ns = u64::from_le_bytes(bytes[12..20].try_into().unwrap());
        let event_count = u64::from_le_bytes(bytes[20..28].try_into().unwrap());
        let venue_mask = u32::from_le_bytes(bytes[28..32].try_into().unwrap());
        let mut reserved = [0u8; 32];
        reserved.copy_from_slice(&bytes[32..64]);

        Ok(TapeHeader {
            magic,
            version,
            flags,
            created_timestamp_ns,
            event_count,
            venue_mask,
            reserved,
        })
    }
}

/// Encodes a [`FeedEvent`] into the provided destination byte slice without heap allocation.
///
/// Returns the number of bytes written.
pub fn encode_event(event: &FeedEvent, out: &mut [u8]) -> Result<usize> {
    if out.len() < MAX_RECORD_SIZE {
        return Err(FeedError::BufferOverflow(out.len()));
    }

    match *event {
        FeedEvent::Snapshot {
            venue_id,
            market_id,
            outcome_id,
            seq,
            timestamp_ns,
            levels,
            num_levels,
        } => {
            out[0] = RECORD_TAG_SNAPSHOT;
            out[1..3].copy_from_slice(&venue_id.to_le_bytes());
            out[3..7].copy_from_slice(&market_id.to_le_bytes());
            out[7..11].copy_from_slice(&outcome_id.to_le_bytes());
            out[11..19].copy_from_slice(&seq.to_le_bytes());
            out[19..27].copy_from_slice(&timestamp_ns.to_le_bytes());
            let count = (num_levels as usize).min(MAX_LEVELS);
            out[27] = count as u8;

            let mut offset = 28;
            for level in &levels[..count] {
                out[offset..offset + 4].copy_from_slice(&level.price.ppm().to_le_bytes());
                offset += 4;
                out[offset..offset + 8].copy_from_slice(&level.size.to_le_bytes());
                offset += 8;
            }
            Ok(offset)
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
            out[0] = RECORD_TAG_DELTA;
            out[1..3].copy_from_slice(&venue_id.to_le_bytes());
            out[3..7].copy_from_slice(&market_id.to_le_bytes());
            out[7..11].copy_from_slice(&outcome_id.to_le_bytes());
            out[11..19].copy_from_slice(&seq.to_le_bytes());
            out[19..27].copy_from_slice(&timestamp_ns.to_le_bytes());
            out[27..31].copy_from_slice(&level.price.ppm().to_le_bytes());
            out[31..39].copy_from_slice(&level.size.to_le_bytes());
            out[39] = if is_delete { 1 } else { 0 };
            Ok(40)
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
            out[0] = RECORD_TAG_TRADE;
            out[1..3].copy_from_slice(&venue_id.to_le_bytes());
            out[3..7].copy_from_slice(&market_id.to_le_bytes());
            out[7..11].copy_from_slice(&outcome_id.to_le_bytes());
            out[11..19].copy_from_slice(&seq.to_le_bytes());
            out[19..27].copy_from_slice(&timestamp_ns.to_le_bytes());
            out[27..31].copy_from_slice(&price.ppm().to_le_bytes());
            out[31..39].copy_from_slice(&size.to_le_bytes());
            out[39] = side.to_u8();
            Ok(40)
        }

        FeedEvent::Heartbeat {
            venue_id,
            timestamp_ns,
        } => {
            out[0] = RECORD_TAG_HEARTBEAT;
            out[1..3].copy_from_slice(&venue_id.to_le_bytes());
            out[3..11].copy_from_slice(&timestamp_ns.to_le_bytes());
            Ok(11)
        }

        FeedEvent::Halt {
            venue_id,
            market_id,
            outcome_id,
            timestamp_ns,
            reason_code,
        } => {
            out[0] = RECORD_TAG_HALT;
            out[1..3].copy_from_slice(&venue_id.to_le_bytes());
            out[3..7].copy_from_slice(&market_id.to_le_bytes());
            if let Some(oid) = outcome_id {
                out[7] = 1;
                out[8..12].copy_from_slice(&oid.to_le_bytes());
            } else {
                out[7] = 0;
                out[8..12].copy_from_slice(&0u32.to_le_bytes());
            }
            out[12..20].copy_from_slice(&timestamp_ns.to_le_bytes());
            out[20] = reason_code;
            Ok(21)
        }
    }
}

/// Decodes a [`FeedEvent`] from a binary record slice directly into caller-provided `out`.
///
/// Returns the number of bytes consumed from `src`.
pub fn decode_event(src: &[u8], out: &mut FeedEvent) -> Result<usize> {
    if src.is_empty() {
        return Err(FeedError::TapeCorrupted("empty record slice"));
    }

    let tag = src[0];
    match tag {
        RECORD_TAG_SNAPSHOT => {
            if src.len() < 28 {
                return Err(FeedError::TapeCorrupted("truncated snapshot record header"));
            }
            let venue_id = u16::from_le_bytes([src[1], src[2]]);
            let market_id = u32::from_le_bytes(src[3..7].try_into().unwrap());
            let outcome_id = u32::from_le_bytes(src[7..11].try_into().unwrap());
            let seq = u64::from_le_bytes(src[11..19].try_into().unwrap());
            let timestamp_ns = u64::from_le_bytes(src[19..27].try_into().unwrap());
            let num_levels = src[27] as usize;

            let expected_len = 28 + num_levels * 12;
            if src.len() < expected_len {
                return Err(FeedError::TapeCorrupted(
                    "truncated snapshot levels payload",
                ));
            }

            let mut levels = [Level {
                price: Prob::CERTAIN,
                size: 0,
            }; MAX_LEVELS];
            let count = num_levels.min(MAX_LEVELS);
            let mut offset = 28;
            for level in levels.iter_mut().take(count) {
                let ppm = u32::from_le_bytes(src[offset..offset + 4].try_into().unwrap());
                offset += 4;
                let size = i64::from_le_bytes(src[offset..offset + 8].try_into().unwrap());
                offset += 8;
                let price =
                    Prob::from_ppm(ppm).map_err(|e| FeedError::InvalidPrice(format!("{e}")))?;
                *level = Level { price, size };
            }

            *out = FeedEvent::Snapshot {
                venue_id,
                market_id,
                outcome_id,
                seq,
                timestamp_ns,
                levels,
                num_levels: count as u8,
            };
            Ok(expected_len)
        }

        RECORD_TAG_DELTA => {
            if src.len() < 40 {
                return Err(FeedError::TapeCorrupted("truncated delta record"));
            }
            let venue_id = u16::from_le_bytes([src[1], src[2]]);
            let market_id = u32::from_le_bytes(src[3..7].try_into().unwrap());
            let outcome_id = u32::from_le_bytes(src[7..11].try_into().unwrap());
            let seq = u64::from_le_bytes(src[11..19].try_into().unwrap());
            let timestamp_ns = u64::from_le_bytes(src[19..27].try_into().unwrap());
            let ppm = u32::from_le_bytes(src[27..31].try_into().unwrap());
            let size = i64::from_le_bytes(src[31..39].try_into().unwrap());
            let is_delete = src[39] != 0;

            let price = Prob::from_ppm(ppm).map_err(|e| FeedError::InvalidPrice(format!("{e}")))?;
            *out = FeedEvent::Delta {
                venue_id,
                market_id,
                outcome_id,
                seq,
                timestamp_ns,
                level: Level { price, size },
                is_delete,
            };
            Ok(40)
        }

        RECORD_TAG_TRADE => {
            if src.len() < 40 {
                return Err(FeedError::TapeCorrupted("truncated trade record"));
            }
            let venue_id = u16::from_le_bytes([src[1], src[2]]);
            let market_id = u32::from_le_bytes(src[3..7].try_into().unwrap());
            let outcome_id = u32::from_le_bytes(src[7..11].try_into().unwrap());
            let seq = u64::from_le_bytes(src[11..19].try_into().unwrap());
            let timestamp_ns = u64::from_le_bytes(src[19..27].try_into().unwrap());
            let ppm = u32::from_le_bytes(src[27..31].try_into().unwrap());
            let size = i64::from_le_bytes(src[31..39].try_into().unwrap());
            let side = TradeSide::from_u8(src[39]);

            let price = Prob::from_ppm(ppm).map_err(|e| FeedError::InvalidPrice(format!("{e}")))?;
            *out = FeedEvent::Trade {
                venue_id,
                market_id,
                outcome_id,
                seq,
                timestamp_ns,
                price,
                size,
                side,
            };
            Ok(40)
        }

        RECORD_TAG_HEARTBEAT => {
            if src.len() < 11 {
                return Err(FeedError::TapeCorrupted("truncated heartbeat record"));
            }
            let venue_id = u16::from_le_bytes([src[1], src[2]]);
            let timestamp_ns = u64::from_le_bytes(src[3..11].try_into().unwrap());
            *out = FeedEvent::Heartbeat {
                venue_id,
                timestamp_ns,
            };
            Ok(11)
        }

        RECORD_TAG_HALT => {
            if src.len() < 21 {
                return Err(FeedError::TapeCorrupted("truncated halt record"));
            }
            let venue_id = u16::from_le_bytes([src[1], src[2]]);
            let market_id = u32::from_le_bytes(src[3..7].try_into().unwrap());
            let has_outcome = src[7] != 0;
            let outcome_id = if has_outcome {
                Some(u32::from_le_bytes(src[8..12].try_into().unwrap()))
            } else {
                None
            };
            let timestamp_ns = u64::from_le_bytes(src[12..20].try_into().unwrap());
            let reason_code = src[20];

            *out = FeedEvent::Halt {
                venue_id,
                market_id,
                outcome_id,
                timestamp_ns,
                reason_code,
            };
            Ok(21)
        }

        _unknown => Err(FeedError::TapeCorrupted("unknown record tag")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_roundtrip() {
        let header = TapeHeader {
            created_timestamp_ns: 1_700_000_000_000,
            event_count: 42,
            venue_mask: 0b11,
            ..Default::default()
        };

        let encoded = header.encode();
        let decoded = TapeHeader::decode(&encoded).unwrap();
        assert_eq!(header, decoded);
    }

    #[test]
    fn test_record_codec_roundtrip() {
        let mut buf = [0u8; MAX_RECORD_SIZE];
        let mut decoded = FeedEvent::Heartbeat {
            venue_id: 0,
            timestamp_ns: 0,
        };

        // Snapshot
        let snap = FeedEvent::snapshot(
            1,
            2,
            3,
            100,
            1_000_000,
            &[
                Level {
                    price: Prob::from_cents(50).unwrap(),
                    size: 1000,
                },
                Level {
                    price: Prob::from_cents(55).unwrap(),
                    size: 2000,
                },
            ],
        );
        let written = encode_event(&snap, &mut buf).unwrap();
        let read = decode_event(&buf[..written], &mut decoded).unwrap();
        assert_eq!(written, read);
        assert_eq!(snap, decoded);

        // Delta
        let delta = FeedEvent::delta(
            1,
            2,
            3,
            101,
            1_000_500,
            Level {
                price: Prob::from_cents(52).unwrap(),
                size: 500,
            },
            false,
        );
        let written = encode_event(&delta, &mut buf).unwrap();
        let read = decode_event(&buf[..written], &mut decoded).unwrap();
        assert_eq!(written, read);
        assert_eq!(delta, decoded);

        // Trade
        let trade = FeedEvent::trade(
            2,
            3,
            4,
            102,
            1_001_000,
            Prob::from_cents(53).unwrap(),
            5000,
            TradeSide::Buy,
        );
        let written = encode_event(&trade, &mut buf).unwrap();
        let read = decode_event(&buf[..written], &mut decoded).unwrap();
        assert_eq!(written, read);
        assert_eq!(trade, decoded);

        // Heartbeat
        let hb = FeedEvent::heartbeat(1, 1_002_000);
        let written = encode_event(&hb, &mut buf).unwrap();
        let read = decode_event(&buf[..written], &mut decoded).unwrap();
        assert_eq!(written, read);
        assert_eq!(hb, decoded);

        // Halt
        let halt = FeedEvent::halt(2, 3, Some(4), 1_003_000, 1);
        let written = encode_event(&halt, &mut buf).unwrap();
        let read = decode_event(&buf[..written], &mut decoded).unwrap();
        assert_eq!(written, read);
        assert_eq!(halt, decoded);
    }
}
