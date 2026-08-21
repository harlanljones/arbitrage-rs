//! Latency and queue modeling for simulated order execution.
//!
//! Real arbitrage fails primarily because signals decay between detection and
//! arrival at the venue matching engine. This module models:
//!
//! 1. **Wire Delay:** One-way network latency from the engine's host to the venue.
//! 2. **Venue Processing Delay:** Time required by the exchange matching engine to process and sequence incoming orders.
//! 3. **Queue Degradation:** Loss of resting liquidity to competing market participants with superior queue priority or faster colocation.

use arbkit_core::{Cents, VenueId};

/// One basis point (0.01%).
const BPS: u64 = 10_000;

/// Maximum number of explicitly configured venues in the fixed lookup table.
pub const MAX_CONFIGURED_VENUES: usize = 32;

/// Latency and queue parameters for a specific venue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LatencyProfile {
    /// One-way wire latency from engine to venue gateway, in nanoseconds.
    pub wire_delay_ns: u64,
    /// Internal venue matching and order sequencing latency, in nanoseconds.
    pub venue_processing_ns: u64,
    /// Percentage of resting book depth (in basis points) assumed consumed
    /// by faster competitors in the queue before this order arrives.
    ///
    /// For example, `2000` (20%) means 20% of resting depth is eaten by
    /// front-running orders, leaving 80% available.
    pub queue_front_run_bps: u32,
}

impl LatencyProfile {
    /// Zero latency with perfect queue priority. Useful for baseline tests.
    pub const ZERO: LatencyProfile = LatencyProfile {
        wire_delay_ns: 0,
        venue_processing_ns: 0,
        queue_front_run_bps: 0,
    };

    /// Create a new latency profile.
    #[inline]
    pub const fn new(
        wire_delay_ns: u64,
        venue_processing_ns: u64,
        queue_front_run_bps: u32,
    ) -> Self {
        Self {
            wire_delay_ns,
            venue_processing_ns,
            queue_front_run_bps,
        }
    }

    /// Profile representing a co-located cross-connect (ultra-low latency).
    ///
    /// Wire delay ~15 µs, venue processing ~25 µs, 5% queue front-run.
    #[inline]
    pub const fn colocated() -> Self {
        Self {
            wire_delay_ns: 15_000,
            venue_processing_ns: 25_000,
            queue_front_run_bps: 500,
        }
    }

    /// Profile representing same-region cloud to cloud (e.g. AWS us-east-1).
    ///
    /// Wire delay ~1.5 ms, venue processing ~1.0 ms, 20% queue front-run.
    #[inline]
    pub const fn regional_cloud() -> Self {
        Self {
            wire_delay_ns: 1_500_000,
            venue_processing_ns: 1_000_000,
            queue_front_run_bps: 2_000,
        }
    }

    /// Profile representing cross-region or public internet routing.
    ///
    /// Wire delay ~35 ms, venue processing ~5 ms, 50% queue front-run.
    #[inline]
    pub const fn cross_region() -> Self {
        Self {
            wire_delay_ns: 35_000_000,
            venue_processing_ns: 5_000_000,
            queue_front_run_bps: 5_000,
        }
    }

    /// Total latency from signal emission to venue execution in nanoseconds.
    #[inline]
    pub const fn total_latency_ns(&self) -> u64 {
        self.wire_delay_ns.saturating_add(self.venue_processing_ns)
    }

    /// Apply the queue degradation model to resting depth.
    ///
    /// Reduces resting depth by [`LatencyProfile::queue_front_run_bps`].
    /// Rounding is pessimistic (floored), guaranteeing that simulated
    /// available depth never overstates reality.
    #[inline]
    pub fn effective_depth(&self, raw_depth: Cents) -> Cents {
        if raw_depth <= 0 {
            return 0;
        }
        let bps = u64::from(self.queue_front_run_bps).min(BPS);
        let remaining_bps = BPS - bps;
        ((raw_depth as u128 * remaining_bps as u128) / BPS as u128) as Cents
    }
}

impl Default for LatencyProfile {
    fn default() -> Self {
        Self::regional_cloud()
    }
}

/// Holds per-venue latency profiles and calculates order arrival times.
#[derive(Debug, Clone)]
pub struct LatencyModel {
    default_profile: LatencyProfile,
    venue_profiles: [(VenueId, LatencyProfile); MAX_CONFIGURED_VENUES],
    configured_count: usize,
}

impl LatencyModel {
    /// Create a new model with a fallback profile for unconfigured venues.
    pub fn new(default_profile: LatencyProfile) -> Self {
        Self {
            default_profile,
            venue_profiles: [(0, LatencyProfile::ZERO); MAX_CONFIGURED_VENUES],
            configured_count: 0,
        }
    }

    /// Set a custom profile for a specific venue.
    pub fn set_venue_profile(&mut self, venue: VenueId, profile: LatencyProfile) {
        for i in 0..self.configured_count {
            if self.venue_profiles[i].0 == venue {
                self.venue_profiles[i].1 = profile;
                return;
            }
        }
        if self.configured_count < MAX_CONFIGURED_VENUES {
            self.venue_profiles[self.configured_count] = (venue, profile);
            self.configured_count += 1;
        }
    }

    /// Builder pattern helper for setting a venue profile.
    pub fn with_venue_profile(mut self, venue: VenueId, profile: LatencyProfile) -> Self {
        self.set_venue_profile(venue, profile);
        self
    }

    /// Retrieve the latency profile for a venue.
    #[inline]
    pub fn profile_for(&self, venue: VenueId) -> LatencyProfile {
        for i in 0..self.configured_count {
            if self.venue_profiles[i].0 == venue {
                return self.venue_profiles[i].1;
            }
        }
        self.default_profile
    }

    /// Compute the simulated arrival timestamp at a venue's matching engine.
    #[inline]
    pub fn arrival_time_ns(&self, detection_timestamp_ns: u64, venue: VenueId) -> u64 {
        let profile = self.profile_for(venue);
        detection_timestamp_ns.saturating_add(profile.total_latency_ns())
    }

    /// Calculate effective resting depth after applying queue position decay.
    #[inline]
    pub fn effective_depth(&self, venue: VenueId, raw_depth: Cents) -> Cents {
        self.profile_for(venue).effective_depth(raw_depth)
    }
}

impl Default for LatencyModel {
    fn default() -> Self {
        Self::new(LatencyProfile::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latency_totals_and_arrival_times() {
        let profile = LatencyProfile::new(1_000_000, 500_000, 2000);
        assert_eq!(profile.total_latency_ns(), 1_500_000);

        let mut model = LatencyModel::new(LatencyProfile::default());
        model.set_venue_profile(1, profile);

        assert_eq!(model.arrival_time_ns(10_000_000, 1), 11_500_000);
        // Fallback for unconfigured venue 2
        assert_eq!(
            model.arrival_time_ns(10_000_000, 2),
            10_000_000 + LatencyProfile::default().total_latency_ns()
        );
    }

    #[test]
    fn queue_degradation_reduces_depth() {
        let profile = LatencyProfile::new(100, 100, 2500); // 25% front-run
        let raw = 100_000;
        assert_eq!(profile.effective_depth(raw), 75_000);

        // 100% front-run eats all depth
        let zero_profile = LatencyProfile::new(100, 100, 10_000);
        assert_eq!(zero_profile.effective_depth(raw), 0);
    }
}
