//! Canonical Event and Market Registry.
//!
//! Maintains the centralized mapping from venue-specific identifiers (Kalshi tickers,
//! Polymarket CLOB token IDs, sportsbook selections) to unified canonical [`MarketId`]
//! and [`OutcomeId`] representations.

use std::collections::HashMap;

use arbkit_core::{Line, MarketId, MarketKind, OutcomeId, VenueId};

use crate::alignment::OutcomeSide;
use crate::error::{MatchError, Result};
use crate::intern::VenueRegistry;
use crate::kalshi::parse_kalshi_ticker;
use crate::lookup::{HotLookupTable, OutcomeRecord};
use crate::team::{CanonicalTeam, Sport};

/// A canonical sporting or prediction contest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalEvent {
    /// Unique canonical event identifier.
    pub id: u32,
    /// Descriptive event title.
    pub name: String,
    /// Sport or league classification.
    pub sport: Sport,
    /// Canonical home team.
    pub home: &'static CanonicalTeam,
    /// Canonical away team.
    pub away: &'static CanonicalTeam,
    /// Optional date identifier string (e.g. "26AUG18").
    pub date_code: Option<String>,
}

/// A canonical market proposition belonging to an event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalMarket {
    /// Unique canonical market identifier.
    pub id: MarketId,
    /// Associated canonical event identifier.
    pub event_id: u32,
    /// Market proposition kind (Moneyline, Spread, Total).
    pub kind: MarketKind,
    /// Associated outcome IDs.
    pub outcomes: Vec<OutcomeId>,
}

/// A single tradeable outcome side within a canonical market.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalOutcome {
    /// Unique canonical outcome identifier.
    pub id: OutcomeId,
    /// Associated canonical market identifier.
    pub market_id: MarketId,
    /// The specific side of the proposition.
    pub side: OutcomeSide,
    /// Human-readable outcome description.
    pub name: String,
}

/// Central registry of all canonical events, markets, and venue mappings.
#[derive(Debug, Clone, Default)]
pub struct CanonicalRegistry {
    events: Vec<CanonicalEvent>,
    markets: Vec<CanonicalMarket>,
    outcomes: Vec<CanonicalOutcome>,
    venue_map: HashMap<(VenueId, String), OutcomeId>,
}

impl CanonicalRegistry {
    /// Create a new, empty canonical registry.
    pub fn new() -> Self {
        Self {
            events: Vec::new(),
            markets: Vec::new(),
            outcomes: Vec::new(),
            venue_map: HashMap::new(),
        }
    }

    /// Register a new canonical event and return its unique `EventId`.
    pub fn create_event(
        &mut self,
        name: &str,
        sport: Sport,
        home: &'static CanonicalTeam,
        away: &'static CanonicalTeam,
        date_code: Option<&str>,
    ) -> u32 {
        let id = self.events.len() as u32;
        self.events.push(CanonicalEvent {
            id,
            name: name.to_string(),
            sport,
            home,
            away,
            date_code: date_code.map(|s| s.to_string()),
        });
        id
    }

    /// Retrieve an event by its canonical ID.
    pub fn get_event(&self, event_id: u32) -> Option<&CanonicalEvent> {
        self.events.get(event_id as usize)
    }

    /// Retrieve a market by its canonical ID.
    pub fn get_market(&self, market_id: MarketId) -> Option<&CanonicalMarket> {
        self.markets.get(market_id as usize)
    }

    /// Retrieve an outcome by its canonical ID.
    pub fn get_outcome(&self, outcome_id: OutcomeId) -> Option<&CanonicalOutcome> {
        self.outcomes.get(outcome_id as usize)
    }

    /// Find an event by sport, date code, home, and away teams.
    pub fn find_event(
        &self,
        sport: Sport,
        date_code: Option<&str>,
        home: &'static CanonicalTeam,
        away: &'static CanonicalTeam,
    ) -> Option<u32> {
        self.events
            .iter()
            .position(|e| {
                e.sport == sport
                    && e.home == home
                    && e.away == away
                    && (date_code.is_none() || e.date_code.as_deref() == date_code)
            })
            .map(|idx| idx as u32)
    }

    /// Create a standard 2-way Moneyline market for an event.
    ///
    /// Returns `(market_id, home_outcome_id, away_outcome_id)`.
    pub fn create_moneyline_market(
        &mut self,
        event_id: u32,
    ) -> Result<(MarketId, OutcomeId, OutcomeId)> {
        let (home_name, away_name) = {
            let event = self
                .get_event(event_id)
                .ok_or(MatchError::EventNotFound(event_id))?;
            (event.home.full_name, event.away.full_name)
        };
        let market_id = self.markets.len() as MarketId;

        let home_outcome_id = self.outcomes.len() as OutcomeId;
        self.outcomes.push(CanonicalOutcome {
            id: home_outcome_id,
            market_id,
            side: OutcomeSide::Home,
            name: format!("{} Moneyline", home_name),
        });

        let away_outcome_id = self.outcomes.len() as OutcomeId;
        self.outcomes.push(CanonicalOutcome {
            id: away_outcome_id,
            market_id,
            side: OutcomeSide::Away,
            name: format!("{} Moneyline", away_name),
        });

        self.markets.push(CanonicalMarket {
            id: market_id,
            event_id,
            kind: MarketKind::Moneyline,
            outcomes: vec![home_outcome_id, away_outcome_id],
        });

        Ok((market_id, home_outcome_id, away_outcome_id))
    }

    /// Create a standard 2-way Spread market for an event normalized to home team handicap.
    ///
    /// Returns `(market_id, home_cover_outcome_id, away_cover_outcome_id)`.
    pub fn create_spread_market(
        &mut self,
        event_id: u32,
        home_line: Line,
    ) -> Result<(MarketId, OutcomeId, OutcomeId)> {
        let (home_name, away_name) = {
            let event = self
                .get_event(event_id)
                .ok_or(MatchError::EventNotFound(event_id))?;
            (event.home.full_name, event.away.full_name)
        };
        let market_id = self.markets.len() as MarketId;

        let home_outcome_id = self.outcomes.len() as OutcomeId;
        self.outcomes.push(CanonicalOutcome {
            id: home_outcome_id,
            market_id,
            side: OutcomeSide::HomeCover,
            name: format!("{} Spread {:+}", home_name, home_line.as_f64()),
        });

        let away_outcome_id = self.outcomes.len() as OutcomeId;
        self.outcomes.push(CanonicalOutcome {
            id: away_outcome_id,
            market_id,
            side: OutcomeSide::AwayCover,
            name: format!("{} Spread {:+}", away_name, home_line.mirrored().as_f64()),
        });

        self.markets.push(CanonicalMarket {
            id: market_id,
            event_id,
            kind: MarketKind::Spread(home_line),
            outcomes: vec![home_outcome_id, away_outcome_id],
        });

        Ok((market_id, home_outcome_id, away_outcome_id))
    }

    /// Create a standard 2-way Total market for an event.
    ///
    /// Returns `(market_id, over_outcome_id, under_outcome_id)`.
    pub fn create_total_market(
        &mut self,
        event_id: u32,
        line: Line,
    ) -> Result<(MarketId, OutcomeId, OutcomeId)> {
        let _event = self
            .get_event(event_id)
            .ok_or(MatchError::EventNotFound(event_id))?;
        let market_id = self.markets.len() as MarketId;

        let over_outcome_id = self.outcomes.len() as OutcomeId;
        self.outcomes.push(CanonicalOutcome {
            id: over_outcome_id,
            market_id,
            side: OutcomeSide::Over,
            name: format!("Over {}", line.as_f64()),
        });

        let under_outcome_id = self.outcomes.len() as OutcomeId;
        self.outcomes.push(CanonicalOutcome {
            id: under_outcome_id,
            market_id,
            side: OutcomeSide::Under,
            name: format!("Under {}", line.as_f64()),
        });

        self.markets.push(CanonicalMarket {
            id: market_id,
            event_id,
            kind: MarketKind::Total(line),
            outcomes: vec![over_outcome_id, under_outcome_id],
        });

        Ok((market_id, over_outcome_id, under_outcome_id))
    }

    /// Map a venue-specific symbol string to a canonical [`OutcomeId`].
    pub fn register_venue_mapping(&mut self, venue: VenueId, symbol: &str, outcome: OutcomeId) {
        self.venue_map
            .insert((venue, symbol.trim().to_string()), outcome);
    }

    /// Lookup the canonical [`OutcomeId`] for a venue and symbol.
    pub fn resolve_venue_outcome(&self, venue: VenueId, symbol: &str) -> Option<OutcomeId> {
        self.venue_map
            .get(&(venue, symbol.trim().to_string()))
            .copied()
    }

    /// Register a Kalshi ticker, creating or linking the canonical event and market as needed.
    pub fn register_kalshi_ticker(&mut self, ticker_str: &str) -> Result<OutcomeId> {
        let parsed = parse_kalshi_ticker(ticker_str)?;

        // Find or create event
        let event_id = if let Some(id) = self.find_event(
            parsed.sport,
            Some(&parsed.date_code),
            parsed.matchup.home,
            parsed.matchup.away,
        ) {
            id
        } else {
            let event_name = format!(
                "{} @ {} {}",
                parsed.matchup.away.code, parsed.matchup.home.code, parsed.date_code
            );
            self.create_event(
                &event_name,
                parsed.sport,
                parsed.matchup.home,
                parsed.matchup.away,
                Some(&parsed.date_code),
            )
        };

        let kind = parsed.market_kind.ok_or_else(|| {
            MatchError::MalformedTicker(
                ticker_str.to_string(),
                "could not determine market kind from ticker",
            )
        })?;

        // Find or create market
        let outcome_id = match kind {
            MarketKind::Moneyline => {
                let target = parsed.target_team.ok_or_else(|| {
                    MatchError::MalformedTicker(
                        ticker_str.to_string(),
                        "moneyline ticker missing target team",
                    )
                })?;
                let (market_id, home_id, away_id) =
                    if let Some(m) = self.find_market(event_id, MarketKind::Moneyline) {
                        let m_obj = &self.markets[m as usize];
                        (m, m_obj.outcomes[0], m_obj.outcomes[1])
                    } else {
                        self.create_moneyline_market(event_id)?
                    };
                let _ = market_id;
                if target == parsed.matchup.home {
                    home_id
                } else if target == parsed.matchup.away {
                    away_id
                } else {
                    return Err(MatchError::UnrecognizedTeam(target.code.to_string()));
                }
            }
            MarketKind::Spread(line) => {
                let target = parsed.target_team.ok_or_else(|| {
                    MatchError::MalformedTicker(
                        ticker_str.to_string(),
                        "spread ticker missing target team",
                    )
                })?;
                let (market_id, home_cover_id, away_cover_id) =
                    if let Some(m) = self.find_market(event_id, MarketKind::Spread(line)) {
                        let m_obj = &self.markets[m as usize];
                        (m, m_obj.outcomes[0], m_obj.outcomes[1])
                    } else {
                        self.create_spread_market(event_id, line)?
                    };
                let _ = market_id;
                if target == parsed.matchup.home {
                    home_cover_id
                } else {
                    away_cover_id
                }
            }
            MarketKind::Total(line) => {
                let is_over = parsed.is_over.ok_or_else(|| {
                    MatchError::MalformedTicker(
                        ticker_str.to_string(),
                        "total ticker missing Over/Under flag",
                    )
                })?;
                let (market_id, over_id, under_id) =
                    if let Some(m) = self.find_market(event_id, MarketKind::Total(line)) {
                        let m_obj = &self.markets[m as usize];
                        (m, m_obj.outcomes[0], m_obj.outcomes[1])
                    } else {
                        self.create_total_market(event_id, line)?
                    };
                let _ = market_id;
                if is_over {
                    over_id
                } else {
                    under_id
                }
            }
        };

        self.register_venue_mapping(VenueRegistry::KALSHI, ticker_str, outcome_id);
        Ok(outcome_id)
    }

    /// Find an existing market for an event matching the given [`MarketKind`].
    pub fn find_market(&self, event_id: u32, kind: MarketKind) -> Option<MarketId> {
        self.markets
            .iter()
            .position(|m| m.event_id == event_id && m.kind == kind)
            .map(|idx| idx as MarketId)
    }

    /// Register a Polymarket binary proposition (Yes/No token IDs) for a canonical market.
    pub fn register_polymarket_tokens(
        &mut self,
        market_id: MarketId,
        yes_token_id: &str,
        no_token_id: &str,
    ) -> Result<()> {
        let market = self
            .get_market(market_id)
            .ok_or(MatchError::MarketNotFound(market_id))?;
        if market.outcomes.len() != 2 {
            return Err(MatchError::InvalidLegCount(market.outcomes.len()));
        }

        let yes_outcome = market.outcomes[0];
        let no_outcome = market.outcomes[1];

        self.register_venue_mapping(VenueRegistry::POLYMARKET, yes_token_id, yes_outcome);
        self.register_venue_mapping(VenueRegistry::POLYMARKET, no_token_id, no_outcome);
        Ok(())
    }

    /// Export the canonical registry into a zero-allocation [`HotLookupTable`].
    pub fn build_hot_lookup_table(&self) -> HotLookupTable {
        let mut table = HotLookupTable::with_capacity(self.outcomes.len());

        for outcome in &self.outcomes {
            let market = &self.markets[outcome.market_id as usize];
            let opposite_outcome_id = if market.outcomes.len() == 2 {
                if market.outcomes[0] == outcome.id {
                    Some(market.outcomes[1])
                } else {
                    Some(market.outcomes[0])
                }
            } else {
                None
            };

            table.set(
                outcome.id,
                OutcomeRecord {
                    market_id: market.id,
                    opposite_outcome_id,
                    market_kind: market.kind,
                    side: outcome.side,
                },
            );
        }

        table
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::team::{lookup_team, Sport};

    #[test]
    fn canonical_registry_flow() {
        let mut registry = CanonicalRegistry::new();

        let bos = lookup_team("Boston Celtics", Some(Sport::Nba)).unwrap();
        let lal = lookup_team("Los Angeles Lakers", Some(Sport::Nba)).unwrap();

        let event_id = registry.create_event("BOS @ LAL", Sport::Nba, lal, bos, Some("26AUG18"));
        let (market_id, home_ml, away_ml) = registry.create_moneyline_market(event_id).unwrap();

        assert_eq!(market_id, 0);
        assert_eq!(home_ml, 0);
        assert_eq!(away_ml, 1);

        // Register Polymarket tokens
        registry
            .register_polymarket_tokens(market_id, "0x1234567890abcdef1", "0x1234567890abcdef2")
            .unwrap();

        assert_eq!(
            registry.resolve_venue_outcome(VenueRegistry::POLYMARKET, "0x1234567890abcdef1"),
            Some(home_ml)
        );
        assert_eq!(
            registry.resolve_venue_outcome(VenueRegistry::POLYMARKET, "0x1234567890abcdef2"),
            Some(away_ml)
        );

        // Build hot table and test zero-allocation queries
        let hot_table = registry.build_hot_lookup_table();
        assert_eq!(hot_table.market_of(home_ml), Some(market_id));
        assert_eq!(hot_table.opposite_of(home_ml), Some(away_ml));
        assert!(hot_table.is_opposite(home_ml, away_ml));
    }

    #[test]
    fn register_kalshi_tickers_links_to_same_canonical_market() {
        let mut registry = CanonicalRegistry::new();

        let bos_outcome = registry
            .register_kalshi_ticker("KXNBAGAME-26AUG18BOSLAL-BOS")
            .unwrap();
        let lal_outcome = registry
            .register_kalshi_ticker("KXNBAGAME-26AUG18BOSLAL-LAL")
            .unwrap();

        let hot_table = registry.build_hot_lookup_table();
        assert_eq!(hot_table.opposite_of(bos_outcome), Some(lal_outcome));
        assert!(hot_table.is_opposite(bos_outcome, lal_outcome));
    }
}
