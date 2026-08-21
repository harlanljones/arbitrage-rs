//! Market alignment and proposition validation.
//!
//! Evaluates whether propositions across different venues represent genuine opposite
//! sides of the exact same event and market condition.
//!
//! For example:
//! - Backing Lakers -3.5 and Celtics +3.5 (in a Celtics home game) are opposite sides
//!   of the canonical `MarketKind::Spread(Line::from_hundredths(350))` market.
//! - Backing Over 220.5 and Under 220.5 are opposite sides of `MarketKind::Total(Line::from_hundredths(22050))`.
//! - Hedging Lakers -3.5 against Celtics +3.0 is a line mismatch that will be rejected.

use arbkit_core::{Line, MarketKind};

use crate::error::{MatchError, Result};
use crate::team::CanonicalTeam;

/// The role or side an outcome plays within a canonical market.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutcomeSide {
    /// Moneyline: Home team wins outright.
    Home,
    /// Moneyline: Away team wins outright.
    Away,
    /// Moneyline: Regulation draw/tie (in 3-way sports such as soccer).
    Draw,
    /// Spread: Home team covers the canonical handicap.
    HomeCover,
    /// Spread: Away team covers the canonical handicap.
    AwayCover,
    /// Total: Combined points/goals score over the line.
    Over,
    /// Total: Combined points/goals score under the line.
    Under,
}

impl OutcomeSide {
    /// The exact mutually exclusive opposite side, if this is a standard binary market.
    #[inline]
    pub const fn opposite(self) -> Option<OutcomeSide> {
        match self {
            OutcomeSide::Home => Some(OutcomeSide::Away),
            OutcomeSide::Away => Some(OutcomeSide::Home),
            OutcomeSide::Draw => None,
            OutcomeSide::HomeCover => Some(OutcomeSide::AwayCover),
            OutcomeSide::AwayCover => Some(OutcomeSide::HomeCover),
            OutcomeSide::Over => Some(OutcomeSide::Under),
            OutcomeSide::Under => Some(OutcomeSide::Over),
        }
    }

    /// Whether this side and `other` are genuine opposite sides of a binary market.
    #[inline]
    pub fn is_opposite_of(self, other: OutcomeSide) -> bool {
        self.opposite() == Some(other)
    }
}

/// Align a moneyline selection to canonical home or away side.
pub fn align_moneyline(
    home: &CanonicalTeam,
    away: &CanonicalTeam,
    bet_team: &CanonicalTeam,
) -> Result<OutcomeSide> {
    if bet_team == home {
        Ok(OutcomeSide::Home)
    } else if bet_team == away {
        Ok(OutcomeSide::Away)
    } else {
        Err(MatchError::UnrecognizedTeam(bet_team.code.to_string()))
    }
}

/// Align a spread proposition to the canonical home-team perspective.
///
/// In sports betting, a spread is traditionally quoted from the perspective of the selected team
/// (e.g. Lakers -3.5 or Celtics +3.5). The canonical representation normalizes all spreads to the
/// home team's handicap:
/// - A bet on Home at line `L` yields canonical line `L` with side `OutcomeSide::HomeCover`.
/// - A bet on Away at line `L` yields canonical line `-L` (`L.mirrored()`) with side `OutcomeSide::AwayCover`.
pub fn align_spread(
    home: &CanonicalTeam,
    away: &CanonicalTeam,
    bet_team: &CanonicalTeam,
    quoted_line: Line,
) -> Result<(MarketKind, OutcomeSide)> {
    if bet_team == home {
        Ok((MarketKind::Spread(quoted_line), OutcomeSide::HomeCover))
    } else if bet_team == away {
        let mirrored = quoted_line.mirrored();
        Ok((MarketKind::Spread(mirrored), OutcomeSide::AwayCover))
    } else {
        Err(MatchError::UnrecognizedTeam(bet_team.code.to_string()))
    }
}

/// Align a total proposition to the canonical `MarketKind::Total` and outcome side.
pub fn align_total(line: Line, is_over: bool) -> (MarketKind, OutcomeSide) {
    (
        MarketKind::Total(line),
        if is_over {
            OutcomeSide::Over
        } else {
            OutcomeSide::Under
        },
    )
}

/// Validate that two market kinds and outcome sides form a valid, mutually exclusive binary pair.
///
/// Returns `Ok(())` if the propositions match. Returns descriptive [`MatchError`]s if:
/// - The market kinds or lines mismatch.
/// - Both legs back the same side.
/// - The sides are incompatible.
pub fn validate_binary_pair(
    kind_a: MarketKind,
    side_a: OutcomeSide,
    kind_b: MarketKind,
    side_b: OutcomeSide,
) -> Result<()> {
    // 1. Verify market kind and line equivalence
    match (kind_a, kind_b) {
        (MarketKind::Moneyline, MarketKind::Moneyline) => {}
        (MarketKind::Spread(line_a), MarketKind::Spread(line_b)) => {
            if line_a != line_b {
                return Err(MatchError::LineMismatch {
                    expected: line_a,
                    actual: line_b,
                });
            }
        }
        (MarketKind::Total(line_a), MarketKind::Total(line_b)) => {
            if line_a != line_b {
                return Err(MatchError::LineMismatch {
                    expected: line_a,
                    actual: line_b,
                });
            }
        }
        _ => {
            return Err(MatchError::MarketKindMismatch {
                expected: kind_a,
                actual: kind_b,
            });
        }
    }

    // 2. Verify that sides are distinct
    if side_a == side_b {
        return Err(MatchError::SameSide(side_a));
    }

    // 3. Verify that sides are complementary opposites
    if side_a.opposite() != Some(side_b) {
        return Err(MatchError::IncompatibleSides(side_a, side_b));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::team::{lookup_team, Sport};

    #[test]
    fn spread_alignment_mirrors_away_bets_to_home_perspective() {
        let home = lookup_team("Boston Celtics", Some(Sport::Nba)).unwrap();
        let away = lookup_team("Los Angeles Lakers", Some(Sport::Nba)).unwrap();

        // Celtics -3.5 (Home -3.5)
        let (kind_home, side_home) =
            align_spread(home, away, home, Line::from_hundredths(-350)).unwrap();
        assert_eq!(kind_home, MarketKind::Spread(Line::from_hundredths(-350)));
        assert_eq!(side_home, OutcomeSide::HomeCover);

        // Lakers +3.5 (Away +3.5) -> Mirrors to Home -3.5
        let (kind_away, side_away) =
            align_spread(home, away, away, Line::from_hundredths(350)).unwrap();
        assert_eq!(kind_away, MarketKind::Spread(Line::from_hundredths(-350)));
        assert_eq!(side_away, OutcomeSide::AwayCover);

        // These two legs must validate as a perfect pair
        assert!(validate_binary_pair(kind_home, side_home, kind_away, side_away).is_ok());
    }

    #[test]
    fn spread_line_mismatch_is_rejected() {
        let home = lookup_team("Boston Celtics", Some(Sport::Nba)).unwrap();
        let away = lookup_team("Los Angeles Lakers", Some(Sport::Nba)).unwrap();

        // Celtics -3.5
        let (kind_a, side_a) = align_spread(home, away, home, Line::from_hundredths(-350)).unwrap();
        // Lakers +3.0 (Mirrors to Celtics -3.0)
        let (kind_b, side_b) = align_spread(home, away, away, Line::from_hundredths(300)).unwrap();

        let res = validate_binary_pair(kind_a, side_a, kind_b, side_b);
        assert!(matches!(res, Err(MatchError::LineMismatch { .. })));
    }

    #[test]
    fn totals_alignment_and_validation() {
        let line = Line::from_hundredths(22050);
        let (kind_over, side_over) = align_total(line, true);
        let (kind_under, side_under) = align_total(line, false);

        assert_eq!(kind_over, MarketKind::Total(line));
        assert_eq!(kind_under, MarketKind::Total(line));
        assert_eq!(side_over, OutcomeSide::Over);
        assert_eq!(side_under, OutcomeSide::Under);

        assert!(validate_binary_pair(kind_over, side_over, kind_under, side_under).is_ok());
    }

    #[test]
    fn same_side_or_cross_kind_rejected() {
        let line = Line::from_hundredths(22050);
        let (kind_over, side_over) = align_total(line, true);

        // Same side
        assert!(matches!(
            validate_binary_pair(kind_over, side_over, kind_over, side_over),
            Err(MatchError::SameSide(OutcomeSide::Over))
        ));

        // Cross market kinds: Total vs Moneyline
        assert!(matches!(
            validate_binary_pair(
                kind_over,
                side_over,
                MarketKind::Moneyline,
                OutcomeSide::Home
            ),
            Err(MatchError::MarketKindMismatch { .. })
        ));
    }
}
