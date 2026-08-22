//! Lock-free SPSC (Single-Producer Single-Consumer) Ring Buffer.
//!
//! A zero-allocation, cacheline-padded queue designed for low-latency handoffs
//! between the network feed ingestion thread and the single-threaded engine hot loop,
//! as well as between the engine and downstream simulator/execution threads.
//!
//! # Safety and Concurrency
//!
//! This implementation is written in 100% safe Rust under `#![forbid(unsafe_code)]`.
//! Multi-threaded synchronization is achieved via turn-sequenced atomic operations
//! with acquire-release semantics. All shared head, tail, and slot structures are
//! padded to 64-byte cachelines (`#[repr(align(64))]`) to eliminate false sharing.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// A trait for types that can be stored and loaded atomically in a lock-free slot.
pub trait AtomicSlot: Default + Send + Sync + 'static {
    /// The message/payload type stored in the slot.
    type Item: Copy;

    /// Stores an item into the atomic slot with relaxed ordering.
    fn store(&self, item: &Self::Item);

    /// Loads an item from the atomic slot with relaxed ordering.
    fn load(&self) -> Self::Item;
}

/// Cacheline-padded wrapper to prevent false sharing between CPU cores.
#[repr(align(64))]
#[derive(Debug)]
pub struct CachePadded<T>(pub T);

impl<T: Default> Default for CachePadded<T> {
    fn default() -> Self {
        Self(T::default())
    }
}

/// An individual turn-sequenced slot in the ring buffer.
#[derive(Debug)]
pub struct RingSlot<S: AtomicSlot> {
    turn: AtomicU64,
    payload: S,
}

impl<S: AtomicSlot> Default for RingSlot<S> {
    fn default() -> Self {
        Self {
            turn: AtomicU64::new(0),
            payload: S::default(),
        }
    }
}

/// Shared ring buffer storage backing the producer and consumer.
#[derive(Debug)]
pub struct RingBuffer<S: AtomicSlot> {
    slots: Vec<CachePadded<RingSlot<S>>>,
    capacity: usize,
    mask: u64,
}

/// Single-producer handle for the SPSC ring buffer.
#[derive(Debug)]
pub struct Producer<S: AtomicSlot> {
    head: u64,
    ring: Arc<RingBuffer<S>>,
}

/// Single-consumer handle for the SPSC ring buffer.
#[derive(Debug)]
pub struct Consumer<S: AtomicSlot> {
    tail: u64,
    ring: Arc<RingBuffer<S>>,
}

/// Creates a new lock-free SPSC ring buffer with the requested power-of-two capacity.
///
/// # Panics
///
/// Panics if `capacity` is 0 or not a power of two.
pub fn spsc_ring<S: AtomicSlot>(capacity: usize) -> (Producer<S>, Consumer<S>) {
    assert!(
        capacity > 0 && capacity.is_power_of_two(),
        "ring buffer capacity must be a power of two"
    );

    let mut slots = Vec::with_capacity(capacity);
    for i in 0..capacity {
        let slot = CachePadded(RingSlot {
            turn: AtomicU64::new(i as u64),
            payload: S::default(),
        });
        slots.push(slot);
    }

    let ring = Arc::new(RingBuffer {
        slots,
        capacity,
        mask: (capacity - 1) as u64,
    });

    let producer = Producer {
        head: 0,
        ring: Arc::clone(&ring),
    };

    let consumer = Consumer { tail: 0, ring };

    (producer, consumer)
}

impl<S: AtomicSlot> Producer<S> {
    /// Attempts to push an item to the ring buffer without blocking or allocating.
    ///
    /// Returns `Ok(())` on success, or `Err(item)` if the buffer is full.
    #[inline]
    pub fn try_push(&mut self, item: S::Item) -> Result<(), S::Item> {
        let idx = (self.head & self.ring.mask) as usize;
        let slot = &self.ring.slots[idx].0;

        let turn = slot.turn.load(Ordering::Acquire);
        if turn == self.head {
            slot.payload.store(&item);
            slot.turn.store(self.head + 1, Ordering::Release);
            self.head += 1;
            Ok(())
        } else {
            Err(item)
        }
    }

    /// Returns the fixed capacity of the ring buffer.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.ring.capacity
    }

    /// Checks if the ring buffer currently has no free space.
    #[inline]
    pub fn is_full(&self) -> bool {
        let idx = (self.head & self.ring.mask) as usize;
        let slot = &self.ring.slots[idx].0;
        slot.turn.load(Ordering::Relaxed) != self.head
    }
}

impl<S: AtomicSlot> Consumer<S> {
    /// Attempts to pop an item from the ring buffer without blocking or allocating.
    ///
    /// Returns `Some(item)` if available, or `None` if the buffer is currently empty.
    #[inline]
    pub fn try_pop(&mut self) -> Option<S::Item> {
        let idx = (self.tail & self.ring.mask) as usize;
        let slot = &self.ring.slots[idx].0;

        let turn = slot.turn.load(Ordering::Acquire);
        if turn == self.tail + 1 {
            let item = slot.payload.load();
            slot.turn
                .store(self.tail + self.ring.capacity as u64, Ordering::Release);
            self.tail += 1;
            Some(item)
        } else {
            None
        }
    }

    /// Returns the fixed capacity of the ring buffer.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.ring.capacity
    }

    /// Checks if the ring buffer is currently empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        let idx = (self.tail & self.ring.mask) as usize;
        let slot = &self.ring.slots[idx].0;
        slot.turn.load(Ordering::Relaxed) != self.tail + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI64, AtomicU32};

    #[derive(Default, Debug)]
    struct SimpleSlot {
        val: AtomicU32,
        extra: AtomicI64,
    }

    impl AtomicSlot for SimpleSlot {
        type Item = (u32, i64);

        fn store(&self, item: &Self::Item) {
            self.val.store(item.0, Ordering::Relaxed);
            self.extra.store(item.1, Ordering::Relaxed);
        }

        fn load(&self) -> Self::Item {
            (
                self.val.load(Ordering::Relaxed),
                self.extra.load(Ordering::Relaxed),
            )
        }
    }

    #[test]
    fn test_spsc_basic_push_pop() {
        let (mut producer, mut consumer) = spsc_ring::<SimpleSlot>(4);
        assert!(consumer.is_empty());
        assert!(!producer.is_full());

        assert!(producer.try_push((10, 100)).is_ok());
        assert!(producer.try_push((20, 200)).is_ok());
        assert!(producer.try_push((30, 300)).is_ok());
        assert!(producer.try_push((40, 400)).is_ok());

        assert!(producer.is_full());
        assert_eq!(producer.try_push((50, 500)), Err((50, 500)));

        assert_eq!(consumer.try_pop(), Some((10, 100)));
        assert_eq!(consumer.try_pop(), Some((20, 200)));

        assert!(producer.try_push((50, 500)).is_ok());
        assert!(producer.try_push((60, 600)).is_ok());
        assert!(producer.is_full());

        assert_eq!(consumer.try_pop(), Some((30, 300)));
        assert_eq!(consumer.try_pop(), Some((40, 400)));
        assert_eq!(consumer.try_pop(), Some((50, 500)));
        assert_eq!(consumer.try_pop(), Some((60, 600)));
        assert_eq!(consumer.try_pop(), None);
        assert!(consumer.is_empty());
    }

    #[test]
    fn test_cross_thread_spsc() {
        let (mut producer, mut consumer) = spsc_ring::<SimpleSlot>(1024);
        let count = 100_000;

        let handle = std::thread::spawn(move || {
            let mut received = 0;
            while received < count {
                if let Some((val, extra)) = consumer.try_pop() {
                    assert_eq!(val as i64, extra);
                    assert_eq!(val, received);
                    received += 1;
                } else {
                    std::hint::spin_loop();
                }
            }
        });

        for i in 0..count {
            while producer.try_push((i, i as i64)).is_err() {
                std::hint::spin_loop();
            }
        }

        handle.join().unwrap();
    }

    /// The ring byte budget the ROADMAP-PNL B1 workstream committed to
    /// watching: `Signal` grew to `MAX_CHUNKS` allocations and the signal
    /// slot carries all of them, so this pins the measured footprint and
    /// the pipeline's ring totals. If a slot type grows past these bounds,
    /// reduce `RING_CAPACITY` in `examples/pipeline.rs` and say so here —
    /// do not let the rings silently eat memory or cache.
    #[test]
    fn test_slot_footprint_budget() {
        use crate::event::{FeedEventSlot, SignalEventSlot};

        // Signal slots are the wide ones: 16 allocations plus a 16-leg plan
        // descriptor, ~1.1 KiB each after the B1/B2 widening.
        assert!(std::mem::size_of::<SignalEventSlot>() <= 1280);
        // Feed slots carry MAX_LEVELS price/size pairs; far narrower.
        assert!(std::mem::size_of::<FeedEventSlot>() <= 256);

        // The pipeline's 8192-slot rings stay in single-digit MiB territory.
        const RING_CAPACITY: usize = 8192;
        let signal_ring_bytes = std::mem::size_of::<SignalEventSlot>() * RING_CAPACITY;
        assert!(signal_ring_bytes <= 16 * 1024 * 1024);
    }
}
