//! Deterministic tape player for backtesting, engine benchmarking, and latency evaluation.

use super::reader::TapeReader;
use crate::error::Result;
use crate::event::FeedEvent;
use arbkit_core::{MarketId, OutcomeId, VenueId};
use std::io::Read;

/// Deterministic replay player over a binary market data tape.
#[derive(Debug)]
pub struct TapePlayer<R: Read> {
    reader: TapeReader<R>,
    venue_filter: Option<VenueId>,
    market_filter: Option<MarketId>,
    outcome_filter: Option<OutcomeId>,
    events_played: u64,
}

impl<R: Read> TapePlayer<R> {
    /// Creates a new [`TapePlayer`] from an underlying [`TapeReader`].
    pub fn new(reader: TapeReader<R>) -> Self {
        Self {
            reader,
            venue_filter: None,
            market_filter: None,
            outcome_filter: None,
            events_played: 0,
        }
    }

    /// Filters replay events by venue ID.
    pub fn with_venue_filter(mut self, venue_id: VenueId) -> Self {
        self.venue_filter = Some(venue_id);
        self
    }

    /// Filters replay events by market ID.
    pub fn with_market_filter(mut self, market_id: MarketId) -> Self {
        self.market_filter = Some(market_id);
        self
    }

    /// Filters replay events by outcome ID.
    pub fn with_outcome_filter(mut self, outcome_id: OutcomeId) -> Self {
        self.outcome_filter = Some(outcome_id);
        self
    }

    /// Advances to the next matching [`FeedEvent`] on the tape.
    ///
    /// Reads into a temporary stack variable with zero heap allocation.
    pub fn next_event(&mut self) -> Result<Option<FeedEvent>> {
        let mut event = FeedEvent::heartbeat(0, 0);
        while self.reader.read_event(&mut event)? {
            if let Some(v) = self.venue_filter {
                if event.venue_id() != v {
                    continue;
                }
            }
            if let Some(m) = self.market_filter {
                if event.market_id() != Some(m) {
                    continue;
                }
            }
            if let Some(o) = self.outcome_filter {
                if event.outcome_id() != Some(o) {
                    continue;
                }
            }

            self.events_played += 1;
            return Ok(Some(event));
        }
        Ok(None)
    }

    /// Streams matching events directly into a caller-provided preallocated buffer.
    ///
    /// Populates up to `target_buffer.len()` events and returns the actual count.
    pub fn play_into(&mut self, target_buffer: &mut [FeedEvent]) -> Result<usize> {
        let mut count = 0;
        for slot in target_buffer.iter_mut() {
            match self.next_event()? {
                Some(event) => {
                    *slot = event;
                    count += 1;
                }
                None => break,
            }
        }
        Ok(count)
    }

    /// Total count of matching events emitted by this player.
    #[inline]
    pub const fn events_played(&self) -> u64 {
        self.events_played
    }

    /// Accesses the underlying [`TapeReader`].
    pub fn reader(&self) -> &TapeReader<R> {
        &self.reader
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tape::writer::TapeWriter;
    use arbkit_core::{Level, Prob};

    #[test]
    fn test_tape_player_filters_and_replay() {
        let mut buffer = Vec::new();
        {
            let mut writer = TapeWriter::new(&mut buffer).unwrap();
            writer
                .write_event(&FeedEvent::snapshot(
                    1,
                    10,
                    100,
                    1,
                    1_000_000,
                    &[Level {
                        price: Prob::from_cents(50).unwrap(),
                        size: 1000,
                    }],
                ))
                .unwrap();
            writer
                .write_event(&FeedEvent::snapshot(
                    2,
                    20,
                    200,
                    1,
                    1_000_500,
                    &[Level {
                        price: Prob::from_cents(52).unwrap(),
                        size: 2000,
                    }],
                ))
                .unwrap();
            writer
                .write_event(&FeedEvent::heartbeat(1, 1_001_000))
                .unwrap();
        }

        let reader = TapeReader::new(buffer.as_slice()).unwrap();
        let mut player = TapePlayer::new(reader).with_venue_filter(1);

        let event1 = player.next_event().unwrap().unwrap();
        assert_eq!(event1.venue_id(), 1);
        assert_eq!(event1.market_id(), Some(10));

        let event2 = player.next_event().unwrap().unwrap();
        assert_eq!(event2.venue_id(), 1);
        assert!(matches!(event2, FeedEvent::Heartbeat { .. }));

        assert_eq!(player.next_event().unwrap(), None);
        assert_eq!(player.events_played(), 2);
    }
}
