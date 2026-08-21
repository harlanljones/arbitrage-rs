//! String interning and venue symbol registration.
//!
//! Feeds receive raw strings (e.g. venue names, ticker strings, contract addresses).
//! These must be interned at the feed boundary so that only numeric identifiers
//! ([`VenueId`], [`arbkit_core::MarketId`], [`arbkit_core::OutcomeId`]) reach the hot path. No strings or dynamic
//! allocations cross into the engine loop.

use std::collections::HashMap;

use arbkit_core::VenueId;

use crate::error::{MatchError, Result};

/// A registry mapping venue names to numeric [`VenueId`]s.
#[derive(Debug, Clone)]
pub struct VenueRegistry {
    name_to_id: HashMap<String, VenueId>,
    id_to_name: Vec<String>,
}

impl VenueRegistry {
    /// Predefined identifier for the Kalshi prediction market venue.
    pub const KALSHI: VenueId = 0;
    /// Predefined identifier for the Polymarket prediction market venue.
    pub const POLYMARKET: VenueId = 1;
    /// Predefined identifier for the DraftKings sportsbook venue.
    pub const DRAFTKINGS: VenueId = 2;
    /// Predefined identifier for the FanDuel sportsbook venue.
    pub const FANDUEL: VenueId = 3;
    /// Predefined identifier for the BetMGM sportsbook venue.
    pub const BETMGM: VenueId = 4;
    /// Predefined identifier for the Caesars sportsbook venue.
    pub const CAESARS: VenueId = 5;
    /// Predefined identifier for the Pinnacle sportsbook venue.
    pub const PINNACLE: VenueId = 6;

    /// Create a new venue registry pre-populated with standard venues.
    pub fn new() -> Self {
        let mut registry = Self {
            name_to_id: HashMap::new(),
            id_to_name: Vec::new(),
        };

        registry.register_fixed(Self::KALSHI, "kalshi");
        registry.register_fixed(Self::POLYMARKET, "polymarket");
        registry.register_fixed(Self::DRAFTKINGS, "draftkings");
        registry.register_fixed(Self::FANDUEL, "fanduel");
        registry.register_fixed(Self::BETMGM, "betmgm");
        registry.register_fixed(Self::CAESARS, "caesars");
        registry.register_fixed(Self::PINNACLE, "pinnacle");

        registry
    }

    fn register_fixed(&mut self, id: VenueId, name: &str) {
        let idx = id as usize;
        if self.id_to_name.len() <= idx {
            self.id_to_name.resize(idx + 1, String::new());
        }
        self.id_to_name[idx] = name.to_string();
        self.name_to_id.insert(name.to_lowercase(), id);
    }

    /// Register a custom venue name and return its assigned [`VenueId`].
    pub fn register(&mut self, name: &str) -> Result<VenueId> {
        let normalized = name.trim().to_lowercase();
        if let Some(&id) = self.name_to_id.get(&normalized) {
            return Ok(id);
        }

        if self.id_to_name.len() >= u16::MAX as usize {
            return Err(MatchError::CapacityExceeded("VenueRegistry"));
        }

        let id = self.id_to_name.len() as VenueId;
        self.id_to_name.push(name.to_string());
        self.name_to_id.insert(normalized, id);
        Ok(id)
    }

    /// Lookup a [`VenueId`] from its name.
    pub fn id_of(&self, name: &str) -> Option<VenueId> {
        let normalized = name.trim().to_lowercase();
        self.name_to_id.get(&normalized).copied()
    }

    /// Lookup the venue name for a given [`VenueId`].
    pub fn name_of(&self, id: VenueId) -> Option<&str> {
        self.id_to_name.get(id as usize).map(|s| s.as_str())
    }
}

impl Default for VenueRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// A string interner mapping arbitrary strings to compact 32-bit symbol IDs.
#[derive(Debug, Clone, Default)]
pub struct StringInterner {
    str_to_id: HashMap<String, u32>,
    id_to_str: Vec<String>,
}

impl StringInterner {
    /// Create an empty string interner.
    pub fn new() -> Self {
        Self {
            str_to_id: HashMap::new(),
            id_to_str: Vec::new(),
        }
    }

    /// Intern a string slice and return its unique 32-bit ID.
    pub fn intern(&mut self, s: &str) -> u32 {
        if let Some(&id) = self.str_to_id.get(s) {
            return id;
        }

        let id = self.id_to_str.len() as u32;
        self.id_to_str.push(s.to_string());
        self.str_to_id.insert(s.to_string(), id);
        id
    }

    /// Resolve an interned 32-bit ID back to its string value.
    pub fn resolve(&self, id: u32) -> Option<&str> {
        self.id_to_str.get(id as usize).map(|s| s.as_str())
    }

    /// Return the number of unique interned strings.
    pub fn len(&self) -> usize {
        self.id_to_str.len()
    }

    /// Return whether the interner is empty.
    pub fn is_empty(&self) -> bool {
        self.id_to_str.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn venue_registry_has_standard_venues() {
        let registry = VenueRegistry::new();
        assert_eq!(registry.id_of("kalshi"), Some(VenueRegistry::KALSHI));
        assert_eq!(
            registry.id_of("polymarket"),
            Some(VenueRegistry::POLYMARKET)
        );
        assert_eq!(
            registry.id_of("draftkings"),
            Some(VenueRegistry::DRAFTKINGS)
        );
        assert_eq!(registry.name_of(VenueRegistry::KALSHI), Some("kalshi"));
    }

    #[test]
    fn venue_registry_registers_new_venues() {
        let mut registry = VenueRegistry::new();
        let id = registry.register("MyCustomVenue").unwrap();
        assert_eq!(registry.id_of("mycustomvenue"), Some(id));
        assert_eq!(registry.id_of("MyCustomVenue"), Some(id));
        assert_eq!(registry.name_of(id), Some("MyCustomVenue"));
    }

    #[test]
    fn string_interner_deduplicates() {
        let mut interner = StringInterner::new();
        let id1 = interner.intern("BOS_LAL_2026");
        let id2 = interner.intern("BOS_LAL_2026");
        let id3 = interner.intern("NYK_MIA_2026");

        assert_eq!(id1, id2);
        assert_ne!(id1, id3);
        assert_eq!(interner.resolve(id1), Some("BOS_LAL_2026"));
        assert_eq!(interner.resolve(id3), Some("NYK_MIA_2026"));
        assert_eq!(interner.len(), 2);
    }
}
