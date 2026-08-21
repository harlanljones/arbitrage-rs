//! Property tests for arbkit-match.

use arbkit_core::{Line, MarketKind};
use arbkit_match::{
    align_spread, align_total, normalize_string, validate_binary_pair, CanonicalTeam, OutcomeSide,
    Sport,
};
use proptest::prelude::*;

static TEST_HOME: CanonicalTeam =
    CanonicalTeam::new(Sport::Nba, "BOS", "Boston Celtics", "Boston", "Celtics");
static TEST_AWAY: CanonicalTeam = CanonicalTeam::new(
    Sport::Nba,
    "LAL",
    "Los Angeles Lakers",
    "Los Angeles",
    "Lakers",
);

proptest! {
    #[test]
    fn string_normalization_is_idempotent(s in "\\PC*") {
        let norm1 = normalize_string(&s);
        let norm2 = normalize_string(&norm1);
        prop_assert_eq!(norm1, norm2);
    }

    #[test]
    fn spread_mirroring_roundtrip(line_val in -10_000i32..10_000i32) {
        let line = Line::from_hundredths(line_val);
        let mirrored = line.mirrored();
        prop_assert_eq!(mirrored.mirrored(), line);
    }

    #[test]
    fn spread_alignment_mirrors_consistently(line_val in -5_000i32..5_000i32) {
        let home_line = Line::from_hundredths(line_val);
        let away_line = home_line.mirrored();

        let (kind_home, side_home) = align_spread(&TEST_HOME, &TEST_AWAY, &TEST_HOME, home_line).unwrap();
        let (kind_away, side_away) = align_spread(&TEST_HOME, &TEST_AWAY, &TEST_AWAY, away_line).unwrap();

        prop_assert_eq!(kind_home, kind_away);
        prop_assert_eq!(side_home, OutcomeSide::HomeCover);
        prop_assert_eq!(side_away, OutcomeSide::AwayCover);
        prop_assert!(validate_binary_pair(kind_home, side_home, kind_away, side_away).is_ok());
    }

    #[test]
    fn totals_alignment_consistency(line_val in 100i32..50_000i32) {
        let line = Line::from_hundredths(line_val);
        let (kind_over, side_over) = align_total(line, true);
        let (kind_under, side_under) = align_total(line, false);

        prop_assert_eq!(kind_over, kind_under);
        prop_assert_eq!(kind_over, MarketKind::Total(line));
        prop_assert_eq!(side_over, OutcomeSide::Over);
        prop_assert_eq!(side_under, OutcomeSide::Under);
        prop_assert!(validate_binary_pair(kind_over, side_over, kind_under, side_under).is_ok());
    }
}
