//! Error types for feed ingestion, venue parsing, and tape replay.

use arbkit_core::VenueId;
use thiserror::Error;

/// Errors produced during feed processing, message parsing, and tape replay.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FeedError {
    /// A parsing failure when deserializing or unpacking venue payloads.
    #[error("parse error: {0}")]
    ParseError(String),

    /// A required field was missing in a venue message frame.
    #[error("missing field: {0}")]
    MissingField(&'static str),

    /// A price representation was invalid or out of permissible bounds.
    #[error("invalid price: {0}")]
    InvalidPrice(String),

    /// A sequence number gap was detected on a venue feed.
    #[error("sequence gap on venue {venue_id}: expected {expected}, received {received}")]
    SequenceGap {
        /// Interned venue identifier.
        venue_id: VenueId,
        /// Expected sequence number.
        expected: u64,
        /// Received sequence number.
        received: u64,
    },

    /// Tape header magic bytes or framing was invalid.
    #[error("invalid tape header: {0}")]
    InvalidTapeHeader(&'static str),

    /// Tape format version is unsupported.
    #[error("unsupported tape version: {0} (expected {1})")]
    UnsupportedTapeVersion(u16, u16),

    /// Tape record is truncated or corrupt.
    #[error("tape corrupted: {0}")]
    TapeCorrupted(&'static str),

    /// Buffer capacity was exceeded during batch replay.
    #[error("buffer capacity exceeded: maximum {0}")]
    BufferOverflow(usize),

    /// An I/O error occurred during tape reading or writing.
    #[error("I/O error: {0}")]
    Io(String),
}

/// Result type shorthand for feed operations.
pub type Result<T> = core::result::Result<T, FeedError>;

impl From<std::io::Error> for FeedError {
    fn from(err: std::io::Error) -> Self {
        FeedError::Io(err.to_string())
    }
}
