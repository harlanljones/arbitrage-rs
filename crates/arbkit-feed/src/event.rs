//! Feed data definitions, event models, and message frames.
//!
//! All types in this module are stack-allocated, `Copy` where applicable, and
//! free of dynamic allocations so they can cross feed boundaries and lock-free
//! ring buffers into the hot loop with zero latency penalty.

use arbkit_core::{Cents, Level, MarketId, OutcomeId, Prob, VenueId, MAX_LEVELS};

/// Aggressor side of an executed trade event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum TradeSide {
    /// Unknown or unspecified trade direction.
    #[default]
    Unknown = 0,
    /// Buy side aggressor (taker bought contracts / took asks).
    Buy = 1,
    /// Sell side aggressor (taker sold contracts / took bids).
    Sell = 2,
}

impl TradeSide {
    /// Constructs [`TradeSide`] from its binary representation.
    #[inline]
    pub const fn from_u8(value: u8) -> Self {
        match value {
            1 => TradeSide::Buy,
            2 => TradeSide::Sell,
            _ => TradeSide::Unknown,
        }
    }

    /// Returns the binary representation of [`TradeSide`].
    #[inline]
    pub const fn to_u8(self) -> u8 {
        self as u8
    }
}

/// A discrete market data event arriving from a venue feed.
///
/// This enum is `Copy` and contains no dynamic allocations. It represents the
/// fundamental event unit transferred across the lock-free boundary into the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedEvent {
    /// A full order book snapshot replacing the local book state.
    Snapshot {
        /// Interned venue identifier.
        venue_id: VenueId,
        /// Interned market identifier.
        market_id: MarketId,
        /// Interned outcome identifier within the market.
        outcome_id: OutcomeId,
        /// Sequence number associated with this snapshot.
        seq: u64,
        /// Ingress timestamp in nanoseconds from Unix epoch.
        timestamp_ns: u64,
        /// Order book levels, best-first.
        levels: [Level; MAX_LEVELS],
        /// Number of valid levels populated in `levels`.
        num_levels: u8,
    },

    /// An incremental delta updating or deleting a price level.
    Delta {
        /// Interned venue identifier.
        venue_id: VenueId,
        /// Interned market identifier.
        market_id: MarketId,
        /// Interned outcome identifier.
        outcome_id: OutcomeId,
        /// Sequence number for this update.
        seq: u64,
        /// Ingress timestamp in nanoseconds.
        timestamp_ns: u64,
        /// The updated price level.
        level: Level,
        /// Flag indicating whether this level was deleted/removed from the book.
        is_delete: bool,
    },

    /// A trade execution event on the venue.
    Trade {
        /// Interned venue identifier.
        venue_id: VenueId,
        /// Interned market identifier.
        market_id: MarketId,
        /// Interned outcome identifier.
        outcome_id: OutcomeId,
        /// Sequence number for this trade event.
        seq: u64,
        /// Ingress timestamp in nanoseconds.
        timestamp_ns: u64,
        /// Execution price.
        price: Prob,
        /// Execution size in whole cents.
        size: Cents,
        /// Aggressor side of the trade.
        side: TradeSide,
    },

    /// Heartbeat / keepalive pulse from a venue feed.
    Heartbeat {
        /// Interned venue identifier.
        venue_id: VenueId,
        /// Ingress timestamp in nanoseconds.
        timestamp_ns: u64,
    },

    /// Trading halt or market suspension notification.
    Halt {
        /// Interned venue identifier.
        venue_id: VenueId,
        /// Interned market identifier.
        market_id: MarketId,
        /// Outcome identifier if outcome-specific, or `None` if market-wide.
        outcome_id: Option<OutcomeId>,
        /// Ingress timestamp in nanoseconds.
        timestamp_ns: u64,
        /// Venue-specific halt reason code (e.g. 0: Unknown, 1: Suspended, 2: Closed).
        reason_code: u8,
    },
}

impl FeedEvent {
    /// Returns the event timestamp in nanoseconds from Unix epoch.
    #[inline]
    pub const fn timestamp_ns(&self) -> u64 {
        match *self {
            FeedEvent::Snapshot { timestamp_ns, .. } => timestamp_ns,
            FeedEvent::Delta { timestamp_ns, .. } => timestamp_ns,
            FeedEvent::Trade { timestamp_ns, .. } => timestamp_ns,
            FeedEvent::Heartbeat { timestamp_ns, .. } => timestamp_ns,
            FeedEvent::Halt { timestamp_ns, .. } => timestamp_ns,
        }
    }

    /// Returns the event timestamp in microseconds from Unix epoch.
    #[inline]
    pub const fn timestamp_micros(&self) -> u64 {
        self.timestamp_ns() / 1_000
    }

    /// Returns the interned venue identifier.
    #[inline]
    pub const fn venue_id(&self) -> VenueId {
        match *self {
            FeedEvent::Snapshot { venue_id, .. } => venue_id,
            FeedEvent::Delta { venue_id, .. } => venue_id,
            FeedEvent::Trade { venue_id, .. } => venue_id,
            FeedEvent::Heartbeat { venue_id, .. } => venue_id,
            FeedEvent::Halt { venue_id, .. } => venue_id,
        }
    }

    /// Returns the interned market identifier if this event is associated with a market.
    #[inline]
    pub const fn market_id(&self) -> Option<MarketId> {
        match *self {
            FeedEvent::Snapshot { market_id, .. }
            | FeedEvent::Delta { market_id, .. }
            | FeedEvent::Trade { market_id, .. }
            | FeedEvent::Halt { market_id, .. } => Some(market_id),
            FeedEvent::Heartbeat { .. } => None,
        }
    }

    /// Returns the interned outcome identifier if this event targets a specific outcome.
    #[inline]
    pub const fn outcome_id(&self) -> Option<OutcomeId> {
        match *self {
            FeedEvent::Snapshot { outcome_id, .. }
            | FeedEvent::Delta { outcome_id, .. }
            | FeedEvent::Trade { outcome_id, .. } => Some(outcome_id),
            FeedEvent::Halt { outcome_id, .. } => outcome_id,
            FeedEvent::Heartbeat { .. } => None,
        }
    }

    /// Returns the sequence number if present on this event.
    #[inline]
    pub const fn seq(&self) -> Option<u64> {
        match *self {
            FeedEvent::Snapshot { seq, .. }
            | FeedEvent::Delta { seq, .. }
            | FeedEvent::Trade { seq, .. } => Some(seq),
            FeedEvent::Heartbeat { .. } | FeedEvent::Halt { .. } => None,
        }
    }

    /// Constructs a [`FeedEvent::Snapshot`] from a slice of levels.
    ///
    /// Copies up to [`MAX_LEVELS`] into the preallocated buffer.
    pub fn snapshot(
        venue_id: VenueId,
        market_id: MarketId,
        outcome_id: OutcomeId,
        seq: u64,
        timestamp_ns: u64,
        levels: &[Level],
    ) -> Self {
        let count = levels.len().min(MAX_LEVELS);
        let mut buffer = [Level {
            price: Prob::CERTAIN,
            size: 0,
        }; MAX_LEVELS];
        buffer[..count].copy_from_slice(&levels[..count]);
        FeedEvent::Snapshot {
            venue_id,
            market_id,
            outcome_id,
            seq,
            timestamp_ns,
            levels: buffer,
            num_levels: count as u8,
        }
    }

    /// Constructs a [`FeedEvent::Delta`].
    pub const fn delta(
        venue_id: VenueId,
        market_id: MarketId,
        outcome_id: OutcomeId,
        seq: u64,
        timestamp_ns: u64,
        level: Level,
        is_delete: bool,
    ) -> Self {
        FeedEvent::Delta {
            venue_id,
            market_id,
            outcome_id,
            seq,
            timestamp_ns,
            level,
            is_delete,
        }
    }

    /// Constructs a [`FeedEvent::Trade`].
    #[allow(clippy::too_many_arguments)]
    pub const fn trade(
        venue_id: VenueId,
        market_id: MarketId,
        outcome_id: OutcomeId,
        seq: u64,
        timestamp_ns: u64,
        price: Prob,
        size: Cents,
        side: TradeSide,
    ) -> Self {
        FeedEvent::Trade {
            venue_id,
            market_id,
            outcome_id,
            seq,
            timestamp_ns,
            price,
            size,
            side,
        }
    }

    /// Constructs a [`FeedEvent::Heartbeat`].
    pub const fn heartbeat(venue_id: VenueId, timestamp_ns: u64) -> Self {
        FeedEvent::Heartbeat {
            venue_id,
            timestamp_ns,
        }
    }

    /// Constructs a [`FeedEvent::Halt`].
    pub const fn halt(
        venue_id: VenueId,
        market_id: MarketId,
        outcome_id: Option<OutcomeId>,
        timestamp_ns: u64,
        reason_code: u8,
    ) -> Self {
        FeedEvent::Halt {
            venue_id,
            market_id,
            outcome_id,
            timestamp_ns,
            reason_code,
        }
    }
}

/// Maximum number of events that can be batched inside a single [`FeedMessage`].
pub const MAX_EVENTS_PER_MESSAGE: usize = 16;

/// High-level feed message container representing an ingested wire frame.
///
/// Holds metadata (venue, sequence, ingress timestamp) along with a preallocated
/// buffer of discrete [`FeedEvent`]s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeedMessage {
    /// Ingress timestamp at the feed boundary in nanoseconds.
    pub ingress_timestamp_ns: u64,
    /// Originating venue identifier.
    pub venue_id: VenueId,
    /// Raw sequence number from the venue envelope (0 if unsequenced).
    pub venue_seq: u64,
    /// Number of valid events stored in `events`.
    pub event_count: u8,
    /// Fixed-size inline buffer of discrete feed events.
    pub events: [FeedEvent; MAX_EVENTS_PER_MESSAGE],
}

impl FeedMessage {
    /// Creates a new empty [`FeedMessage`].
    pub const fn new(venue_id: VenueId, ingress_timestamp_ns: u64, venue_seq: u64) -> Self {
        FeedMessage {
            ingress_timestamp_ns,
            venue_id,
            venue_seq,
            event_count: 0,
            events: [FeedEvent::Heartbeat {
                venue_id,
                timestamp_ns: ingress_timestamp_ns,
            }; MAX_EVENTS_PER_MESSAGE],
        }
    }

    /// Constructs a [`FeedMessage`] wrapping a single [`FeedEvent`].
    pub fn from_event(event: FeedEvent) -> Self {
        let mut msg = Self::new(
            event.venue_id(),
            event.timestamp_ns(),
            event.seq().unwrap_or(0),
        );
        msg.push(event);
        msg
    }

    /// Pushes a [`FeedEvent`] into the message buffer if capacity allows.
    ///
    /// Returns `true` if the event was successfully added, or `false` if the buffer is full.
    pub fn push(&mut self, event: FeedEvent) -> bool {
        if (self.event_count as usize) < MAX_EVENTS_PER_MESSAGE {
            self.events[self.event_count as usize] = event;
            self.event_count += 1;
            true
        } else {
            false
        }
    }

    /// Returns a slice of the populated [`FeedEvent`]s.
    #[inline]
    pub fn events(&self) -> &[FeedEvent] {
        &self.events[..self.event_count as usize]
    }

    /// Returns the number of events in this message.
    #[inline]
    pub const fn len(&self) -> usize {
        self.event_count as usize
    }

    /// Checks if this message contains zero events.
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.event_count == 0
    }
}
