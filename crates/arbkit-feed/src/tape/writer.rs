//! High-performance binary tape writer for deterministic market feed recording.

use super::format::{encode_event, TapeHeader, MAX_RECORD_SIZE};
use crate::error::Result;
use crate::event::FeedEvent;
use std::io::Write;

/// Binary market data tape recorder writing timestamped [`FeedEvent`] streams.
#[derive(Debug)]
pub struct TapeWriter<W: Write> {
    writer: W,
    header: TapeHeader,
    events_written: u64,
}

impl<W: Write> TapeWriter<W> {
    /// Creates a new [`TapeWriter`] wrapping an output stream and writes the default header.
    pub fn new(writer: W) -> Result<Self> {
        let header = TapeHeader::default();
        Self::with_header(writer, header)
    }

    /// Creates a new [`TapeWriter`] with a specific [`TapeHeader`].
    pub fn with_header(mut writer: W, header: TapeHeader) -> Result<Self> {
        let encoded_header = header.encode();
        writer.write_all(&encoded_header)?;
        Ok(Self {
            writer,
            header,
            events_written: 0,
        })
    }

    /// Appends a single [`FeedEvent`] to the tape.
    ///
    /// Encodes into a stack-allocated buffer with zero dynamic allocations.
    pub fn write_event(&mut self, event: &FeedEvent) -> Result<()> {
        let mut buf = [0u8; MAX_RECORD_SIZE];
        let bytes_written = encode_event(event, &mut buf)?;
        self.writer.write_all(&buf[..bytes_written])?;
        self.events_written += 1;
        Ok(())
    }

    /// Appends a slice of [`FeedEvent`]s in batch.
    pub fn write_batch(&mut self, events: &[FeedEvent]) -> Result<usize> {
        for event in events {
            self.write_event(event)?;
        }
        Ok(events.len())
    }

    /// Flushes the underlying I/O writer.
    pub fn flush(&mut self) -> Result<()> {
        self.writer.flush()?;
        Ok(())
    }

    /// Returns the total count of events written to this tape.
    #[inline]
    pub const fn events_written(&self) -> u64 {
        self.events_written
    }

    /// Returns a reference to the written tape header.
    #[inline]
    pub const fn header(&self) -> &TapeHeader {
        &self.header
    }

    /// Unwraps the inner writer.
    pub fn into_inner(self) -> W {
        self.writer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::TradeSide;
    use arbkit_core::Prob;

    #[test]
    fn test_tape_writer_output() {
        let mut buffer = Vec::new();
        let mut writer = TapeWriter::new(&mut buffer).unwrap();

        let event1 = FeedEvent::heartbeat(1, 100_000);
        let event2 = FeedEvent::trade(
            1,
            2,
            3,
            10,
            100_500,
            Prob::from_cents(50).unwrap(),
            1000,
            TradeSide::Buy,
        );

        writer.write_event(&event1).unwrap();
        writer.write_event(&event2).unwrap();
        writer.flush().unwrap();

        assert_eq!(writer.events_written(), 2);
        assert!(buffer.len() > 64);
    }
}
