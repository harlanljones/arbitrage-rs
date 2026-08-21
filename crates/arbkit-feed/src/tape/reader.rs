//! High-performance binary tape reader for zero-allocation event replay.

use super::format::{
    decode_event, TapeHeader, RECORD_TAG_DELTA, RECORD_TAG_HALT, RECORD_TAG_HEARTBEAT,
    RECORD_TAG_SNAPSHOT, RECORD_TAG_TRADE, TAPE_HEADER_SIZE,
};
use crate::error::{FeedError, Result};
use crate::event::FeedEvent;
use std::io::Read;

/// Binary market data tape reader streaming recorded [`FeedEvent`]s.
#[derive(Debug)]
pub struct TapeReader<R: Read> {
    reader: R,
    header: TapeHeader,
    events_read: u64,
}

impl<R: Read> TapeReader<R> {
    /// Constructs a [`TapeReader`], reading and validating the binary header from the stream.
    pub fn new(mut reader: R) -> Result<Self> {
        let mut header_bytes = [0u8; TAPE_HEADER_SIZE];
        reader.read_exact(&mut header_bytes)?;
        let header = TapeHeader::decode(&header_bytes)?;
        Ok(Self {
            reader,
            header,
            events_read: 0,
        })
    }

    /// Accesses the validated [`TapeHeader`].
    #[inline]
    pub const fn header(&self) -> &TapeHeader {
        &self.header
    }

    /// Reads the next [`FeedEvent`] into the caller-provided destination with zero heap allocations.
    ///
    /// Returns `Ok(true)` if an event was read, or `Ok(false)` on clean end of stream.
    pub fn read_event(&mut self, out: &mut FeedEvent) -> Result<bool> {
        let mut tag_buf = [0u8; 1];
        match self.reader.read_exact(&mut tag_buf) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(false),
            Err(e) => return Err(e.into()),
        }

        let tag = tag_buf[0];
        let mut buf = [0u8; 128];
        buf[0] = tag;

        let payload_len = match tag {
            RECORD_TAG_SNAPSHOT => {
                // Read 27 bytes (fixed snapshot fields through num_levels)
                self.reader.read_exact(&mut buf[1..28])?;
                let num_levels = buf[27] as usize;
                let levels_bytes = num_levels * 12;
                if levels_bytes > 0 {
                    self.reader.read_exact(&mut buf[28..28 + levels_bytes])?;
                }
                28 + levels_bytes
            }
            RECORD_TAG_DELTA => {
                self.reader.read_exact(&mut buf[1..40])?;
                40
            }
            RECORD_TAG_TRADE => {
                self.reader.read_exact(&mut buf[1..40])?;
                40
            }
            RECORD_TAG_HEARTBEAT => {
                self.reader.read_exact(&mut buf[1..11])?;
                11
            }
            RECORD_TAG_HALT => {
                self.reader.read_exact(&mut buf[1..21])?;
                21
            }
            _ => return Err(FeedError::TapeCorrupted("unrecognized record tag")),
        };

        decode_event(&buf[..payload_len], out)?;
        self.events_read += 1;
        Ok(true)
    }

    /// Reads the next [`FeedEvent`] as an `Option<FeedEvent>`.
    pub fn read_next(&mut self) -> Result<Option<FeedEvent>> {
        let mut event = FeedEvent::heartbeat(0, 0);
        if self.read_event(&mut event)? {
            Ok(Some(event))
        } else {
            Ok(None)
        }
    }

    /// Reads up to `buf.len()` recorded events into a preallocated slice in batch.
    ///
    /// Guarantees zero heap allocation throughout the entire batch iteration.
    pub fn read_batch(&mut self, buf: &mut [FeedEvent]) -> Result<usize> {
        let mut count = 0;
        for slot in buf.iter_mut() {
            if !self.read_event(slot)? {
                break;
            }
            count += 1;
        }
        Ok(count)
    }

    /// Returns the number of events read from this tape so far.
    #[inline]
    pub const fn events_read(&self) -> u64 {
        self.events_read
    }

    /// Unwraps the inner reader.
    pub fn into_inner(self) -> R {
        self.reader
    }
}
