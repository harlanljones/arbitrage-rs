//! High-dynamic-range, fixed-bucket latency histogram.
//!
//! Provides sub-microsecond resolution for ultra-low latency profiling (0 to 10 µs in 10 ns bins,
//! up to 100 ms in multi-tiered linear buckets) without heap allocations during measurement.
//!
//! ## Percentile semantics: all-events, not signal-hits-only
//!
//! [`Engine::process_event`](crate::engine::Engine::process_event) records exactly one
//! sample into the engine's histogram for *every* feed event it processes — snapshots,
//! deltas, trades, halts, and heartbeats alike — not only for the events that end up
//! emitting an arbitrage [`SignalEvent`](crate::event::SignalEvent). This changed from
//! an earlier version where the record call sat inside the signal-emitting branch, so
//! `p50`/`p90`/`p99`/`p999` here describe hot-loop service time across the whole event
//! stream, dominated by the (much more common) no-signal case.
//!
//! This is a different quantity than "latency of the events that produced a signal."
//! That older, narrower metric is still recoverable: `Engine::stats().signals_emitted`
//! gives the count of signal-emitting events independent of the histogram, so the two
//! together (`stats().events_processed`, `stats().signals_emitted`, and this histogram)
//! tell you the signal hit-rate without needing a second histogram. If a caller needs
//! the *distribution* of signal-hit latency specifically (not just its rate), it must
//! keep its own separate [`LatencyHistogram`] and record into it only where the caller
//! observes `process_event` return `Some`, exactly as the signal-only path used to.

/// Number of preallocated histogram bins.
pub const NUM_BINS: usize = 4601;

/// Fixed-capacity latency histogram.
#[derive(Debug, Clone)]
pub struct LatencyHistogram {
    bins: [u64; NUM_BINS],
    count: u64,
    sum_ns: u128,
    min_ns: u64,
    max_ns: u64,
}

impl Default for LatencyHistogram {
    fn default() -> Self {
        Self::new()
    }
}

impl LatencyHistogram {
    /// Creates a new, empty latency histogram with all bins zeroed.
    pub const fn new() -> Self {
        Self {
            bins: [0; NUM_BINS],
            count: 0,
            sum_ns: 0,
            min_ns: u64::MAX,
            max_ns: 0,
        }
    }

    /// Maps a latency in nanoseconds to its corresponding bin index.
    #[inline]
    pub const fn latency_to_bin(latency_ns: u64) -> usize {
        if latency_ns < 10_000 {
            (latency_ns / 10) as usize
        } else if latency_ns < 100_000 {
            1000 + ((latency_ns - 10_000) / 100) as usize
        } else if latency_ns < 1_000_000 {
            1900 + ((latency_ns - 100_000) / 1_000) as usize
        } else if latency_ns < 10_000_000 {
            2800 + ((latency_ns - 1_000_000) / 10_000) as usize
        } else if latency_ns < 100_000_000 {
            3700 + ((latency_ns - 10_000_000) / 100_000) as usize
        } else {
            4600
        }
    }

    /// Maps a bin index back to the nominal latency value in nanoseconds.
    #[inline]
    pub const fn bin_to_latency(bin: usize) -> u64 {
        if bin < 1000 {
            (bin as u64) * 10
        } else if bin < 1900 {
            10_000 + ((bin - 1000) as u64) * 100
        } else if bin < 2800 {
            100_000 + ((bin - 1900) as u64) * 1_000
        } else if bin < 3700 {
            1_000_000 + ((bin - 2800) as u64) * 10_000
        } else if bin < 4600 {
            10_000_000 + ((bin - 3700) as u64) * 100_000
        } else {
            100_000_000
        }
    }

    /// Records a latency observation in nanoseconds.
    #[inline]
    pub fn record(&mut self, latency_ns: u64) {
        let bin = Self::latency_to_bin(latency_ns);
        self.bins[bin] += 1;
        self.count += 1;
        self.sum_ns += latency_ns as u128;

        if latency_ns < self.min_ns {
            self.min_ns = latency_ns;
        }
        if latency_ns > self.max_ns {
            self.max_ns = latency_ns;
        }
    }

    /// Returns the total number of recorded observations.
    #[inline]
    pub const fn count(&self) -> u64 {
        self.count
    }

    /// Returns the minimum recorded latency in nanoseconds, or `None` if empty.
    #[inline]
    pub const fn min_ns(&self) -> Option<u64> {
        if self.count > 0 {
            Some(self.min_ns)
        } else {
            None
        }
    }

    /// Returns the maximum recorded latency in nanoseconds, or `None` if empty.
    #[inline]
    pub const fn max_ns(&self) -> Option<u64> {
        if self.count > 0 {
            Some(self.max_ns)
        } else {
            None
        }
    }

    /// Returns the arithmetic mean latency in nanoseconds, or `None` if empty.
    #[inline]
    pub fn mean_ns(&self) -> Option<u64> {
        if self.count > 0 {
            Some((self.sum_ns / self.count as u128) as u64)
        } else {
            None
        }
    }

    /// Computes the estimated percentile latency in nanoseconds (0.0 to 1.0).
    pub fn percentile(&self, p: f64) -> u64 {
        if self.count == 0 {
            return 0;
        }

        let fraction = p.clamp(0.0, 1.0);
        let target = ((self.count as f64) * fraction).ceil() as u64;
        let target = target.max(1);

        let mut accumulated = 0;
        for (bin, &count) in self.bins.iter().enumerate() {
            accumulated += count;
            if accumulated >= target {
                return Self::bin_to_latency(bin);
            }
        }

        Self::bin_to_latency(NUM_BINS - 1)
    }

    /// Returns the 50th percentile (median) latency in nanoseconds.
    #[inline]
    pub fn p50(&self) -> u64 {
        self.percentile(0.50)
    }

    /// Returns the 90th percentile latency in nanoseconds.
    #[inline]
    pub fn p90(&self) -> u64 {
        self.percentile(0.90)
    }

    /// Returns the 99th percentile latency in nanoseconds.
    #[inline]
    pub fn p99(&self) -> u64 {
        self.percentile(0.99)
    }

    /// Returns the 99.9th percentile latency in nanoseconds.
    #[inline]
    pub fn p999(&self) -> u64 {
        self.percentile(0.999)
    }

    /// Clears and resets all histogram counters.
    pub fn reset(&mut self) {
        *self = Self::new();
    }
}

/// Summary representation of latency metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LatencySummary {
    /// Number of samples.
    pub count: u64,
    /// Minimum latency in nanoseconds.
    pub min_ns: u64,
    /// Mean latency in nanoseconds.
    pub mean_ns: u64,
    /// 50th percentile (median) in nanoseconds.
    pub p50_ns: u64,
    /// 90th percentile in nanoseconds.
    pub p90_ns: u64,
    /// 99th percentile in nanoseconds.
    pub p99_ns: u64,
    /// 99.9th percentile in nanoseconds.
    pub p999_ns: u64,
    /// Maximum latency in nanoseconds.
    pub max_ns: u64,
}

impl LatencyHistogram {
    /// Returns a consolidated summary of the recorded latencies.
    pub fn summary(&self) -> LatencySummary {
        LatencySummary {
            count: self.count,
            min_ns: self.min_ns().unwrap_or(0),
            mean_ns: self.mean_ns().unwrap_or(0),
            p50_ns: self.p50(),
            p90_ns: self.p90(),
            p99_ns: self.p99(),
            p999_ns: self.p999(),
            max_ns: self.max_ns().unwrap_or(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_histogram_recording_and_percentiles() {
        let mut hist = LatencyHistogram::new();
        assert_eq!(hist.count(), 0);
        assert_eq!(hist.min_ns(), None);
        assert_eq!(hist.max_ns(), None);

        // Record 100 samples at 500 ns (0.5 us)
        for _ in 0..100 {
            hist.record(500);
        }

        // Record 1 sample at 20_000 ns (20 us)
        hist.record(20_000);

        assert_eq!(hist.count(), 101);
        assert_eq!(hist.min_ns(), Some(500));
        assert_eq!(hist.max_ns(), Some(20_000));
        assert_eq!(hist.p50(), 500);
        assert_eq!(hist.p99(), 500);
        assert_eq!(hist.p999(), 20_000);

        let summary = hist.summary();
        assert_eq!(summary.count, 101);
        assert_eq!(summary.min_ns, 500);
        assert_eq!(summary.max_ns, 20_000);
    }
}
