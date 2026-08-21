//! Integration tests for arbkit-match covering team normalization, market alignment,
//! venue registry, Kalshi ticker parsing, and hot path lookup tables.

use arbkit_core::{Line, MarketKind};
use arbkit_match::{
    align_spread, align_total, lookup_team, parse_kalshi_ticker, parse_matchup,
    validate_binary_pair, CanonicalRegistry, MatchError, OutcomeSide, Sport, VenueRegistry,
};

#[test]
fn test_team_normalization_and_aliases() {
    // NBA aliases
    let bos = lookup_team("Boston Celtics", Some(Sport::Nba)).unwrap();
    assert_eq!(bos.code, "BOS");
    assert_eq!(bos.mascot, "Celtics");

    let lal = lookup_team("LA Lakers", Some(Sport::Nba)).unwrap();
    assert_eq!(lal.code, "LAL");

    let gsw = lookup_team("Golden State", Some(Sport::Nba)).unwrap();
    assert_eq!(gsw.code, "GSW");

    let phi = lookup_team("Sixers", Some(Sport::Nba)).unwrap();
    assert_eq!(phi.code, "PHI");

    // NFL aliases
    let kc = lookup_team("Chiefs", Some(Sport::Nfl)).unwrap();
    assert_eq!(kc.code, "KC");

    let sf = lookup_team("49ers", Some(Sport::Nfl)).unwrap();
    assert_eq!(sf.code, "SF");

    let gb = lookup_team("Green Bay Packers", Some(Sport::Nfl)).unwrap();
    assert_eq!(gb.code, "GB");

    // MLB aliases
    let nyy = lookup_team("Yankees", Some(Sport::Mlb)).unwrap();
    assert_eq!(nyy.code, "NYY");

    let lad = lookup_team("Dodgers", Some(Sport::Mlb)).unwrap();
    assert_eq!(lad.code, "LAD");

    // Unknown team
    assert!(matches!(
        lookup_team("NonExistentFC", None),
        Err(MatchError::UnrecognizedTeam(_))
    ));
}

#[test]
fn test_matchup_parsing() {
    let m1 = parse_matchup("BOS @ LAL", Some(Sport::Nba)).unwrap();
    assert_eq!(m1.away.code, "BOS");
    assert_eq!(m1.home.code, "LAL");

    let m2 = parse_matchup("Boston Celtics at Los Angeles Lakers", Some(Sport::Nba)).unwrap();
    assert_eq!(m2.away.code, "BOS");
    assert_eq!(m2.home.code, "LAL");

    let m3 = parse_matchup("LAL vs. BOS", Some(Sport::Nba)).unwrap();
    assert_eq!(m3.home.code, "LAL");
    assert_eq!(m3.away.code, "BOS");

    let m4 = parse_matchup("Los Angeles Lakers vs Boston Celtics", Some(Sport::Nba)).unwrap();
    assert_eq!(m4.home.code, "LAL");
    assert_eq!(m4.away.code, "BOS");
}

#[test]
fn test_kalshi_ticker_parsing() {
    // Moneyline ticker
    let t1 = parse_kalshi_ticker("KXNBAGAME-26AUG18BOSLAL-BOS").unwrap();
    assert_eq!(t1.sport, Sport::Nba);
    assert_eq!(t1.date_code, "26AUG18");
    assert_eq!(t1.matchup.away.code, "BOS");
    assert_eq!(t1.matchup.home.code, "LAL");
    assert_eq!(t1.market_kind, Some(MarketKind::Moneyline));
    assert_eq!(t1.target_team.unwrap().code, "BOS");

    // Spread ticker
    let t2 = parse_kalshi_ticker("KXNBASPREAD-26AUG18BOSLAL-BOS35").unwrap();
    assert_eq!(t2.sport, Sport::Nba);
    assert_eq!(t2.target_team.unwrap().code, "BOS");
    // BOS is away, so BOS +3.5 mirrors to Home (LAL) -3.5 => Line(-350)
    assert_eq!(
        t2.market_kind,
        Some(MarketKind::Spread(Line::from_hundredths(-350)))
    );

    // Total ticker
    let t3 = parse_kalshi_ticker("KXNBATOTAL-26AUG18BOSLAL-2205O").unwrap();
    assert_eq!(t3.sport, Sport::Nba);
    assert_eq!(t3.is_over, Some(true));
    assert_eq!(
        t3.market_kind,
        Some(MarketKind::Total(Line::from_hundredths(22050)))
    );
}

#[test]
fn test_spread_mirroring_and_validation() {
    let home = lookup_team("Boston Celtics", Some(Sport::Nba)).unwrap();
    let away = lookup_team("Los Angeles Lakers", Some(Sport::Nba)).unwrap();

    // Venue 1 quotes: Celtics -3.5
    let (kind1, side1) = align_spread(home, away, home, Line::from_hundredths(-350)).unwrap();

    // Venue 2 quotes: Lakers +3.5
    let (kind2, side2) = align_spread(home, away, away, Line::from_hundredths(350)).unwrap();

    // Both should normalize to the same MarketKind and opposite sides
    assert_eq!(kind1, kind2);
    assert_eq!(side1, OutcomeSide::HomeCover);
    assert_eq!(side2, OutcomeSide::AwayCover);
    assert!(validate_binary_pair(kind1, side1, kind2, side2).is_ok());

    // Mismatched line: Lakers +4.0
    let (kind3, side3) = align_spread(home, away, away, Line::from_hundredths(400)).unwrap();
    assert!(matches!(
        validate_binary_pair(kind1, side1, kind3, side3),
        Err(MatchError::LineMismatch { .. })
    ));
}

#[test]
fn test_total_mirroring_and_validation() {
    let line = Line::from_hundredths(22050);
    let (kind_over, side_over) = align_total(line, true);
    let (kind_under, side_under) = align_total(line, false);

    assert_eq!(kind_over, kind_under);
    assert_eq!(side_over, OutcomeSide::Over);
    assert_eq!(side_under, OutcomeSide::Under);
    assert!(validate_binary_pair(kind_over, side_over, kind_under, side_under).is_ok());

    // Different total lines mismatch
    let line_diff = Line::from_hundredths(22100);
    let (kind_over_diff, side_over_diff) = align_total(line_diff, true);
    assert!(matches!(
        validate_binary_pair(kind_over, side_over, kind_over_diff, side_over_diff),
        Err(MatchError::LineMismatch { .. })
    ));
}

#[test]
fn test_canonical_registry_cross_venue_setup() {
    let mut registry = CanonicalRegistry::new();

    let bos_outcome = registry
        .register_kalshi_ticker("KXNBAGAME-26AUG18BOSLAL-BOS")
        .unwrap();
    let lal_outcome = registry
        .register_kalshi_ticker("KXNBAGAME-26AUG18BOSLAL-LAL")
        .unwrap();

    let market_id = registry.get_outcome(bos_outcome).unwrap().market_id;

    // Register Polymarket clob tokens on the same market
    registry
        .register_polymarket_tokens(market_id, "token_lal_win", "token_bos_win")
        .unwrap();

    // Register Sportsbook selection ID
    registry.register_venue_mapping(VenueRegistry::DRAFTKINGS, "dk_sel_bos_ml", bos_outcome);
    registry.register_venue_mapping(VenueRegistry::DRAFTKINGS, "dk_sel_lal_ml", lal_outcome);

    // Feed boundary symbol lookups
    assert_eq!(
        registry.resolve_venue_outcome(VenueRegistry::KALSHI, "KXNBAGAME-26AUG18BOSLAL-BOS"),
        Some(bos_outcome)
    );
    assert_eq!(
        registry.resolve_venue_outcome(VenueRegistry::POLYMARKET, "token_bos_win"),
        Some(bos_outcome)
    );
    assert_eq!(
        registry.resolve_venue_outcome(VenueRegistry::DRAFTKINGS, "dk_sel_bos_ml"),
        Some(bos_outcome)
    );

    // Build zero-allocation hot lookup table
    let hot = registry.build_hot_lookup_table();
    assert_eq!(hot.market_of(bos_outcome), Some(market_id));
    assert_eq!(hot.opposite_of(bos_outcome), Some(lal_outcome));
    assert!(hot.is_opposite(bos_outcome, lal_outcome));
    assert!(!hot.is_opposite(bos_outcome, bos_outcome));
}
