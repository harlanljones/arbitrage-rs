//! Kalshi event and market ticker parser.
//!
//! Kalshi uses structured tickers such as:
//! - Event / Series: `KXNBAGAME-26AUG18BOSLAL`
//! - Moneyline outcome: `KXNBAGAME-26AUG18BOSLAL-BOS`
//! - Spread outcome: `KXNBASPREAD-26AUG18BOSLAL-BOS35`
//! - Total outcome: `KXNBATOTAL-26AUG18BOSLAL-2205O`

use arbkit_core::{Line, MarketKind};

use crate::error::{MatchError, Result};
use crate::team::{lookup_team, CanonicalTeam, Matchup, Sport};

/// A parsed Kalshi market ticker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KalshiTicker {
    /// Sport or league.
    pub sport: Sport,
    /// Raw date code (e.g. "26AUG18").
    pub date_code: String,
    /// Matchup (home and away teams).
    pub matchup: Matchup,
    /// The market kind (Moneyline, Spread, Total) if determinable from ticker.
    pub market_kind: Option<MarketKind>,
    /// Specific outcome target team if applicable (e.g. "BOS").
    pub target_team: Option<&'static CanonicalTeam>,
    /// For totals, whether this ticker represents the Over (`true`) or Under (`false`).
    pub is_over: Option<bool>,
}

/// Parse a Kalshi ticker string into structured event and market information.
pub fn parse_kalshi_ticker(ticker: &str) -> Result<KalshiTicker> {
    let parts: Vec<&str> = ticker.split('-').collect();
    if parts.len() < 2 {
        return Err(MatchError::MalformedTicker(
            ticker.to_string(),
            "expected at least series and event parts separated by '-'",
        ));
    }

    let series_part = parts[0];
    let event_part = parts[1];
    let outcome_part = parts.get(2).copied();

    // 1. Parse series part (e.g., "KXNBAGAME", "KXNBASPREAD", "KXNBATOTAL")
    let (sport, kind_hint) = parse_series_prefix(series_part)?;

    // 2. Parse event part (e.g., "26AUG18BOSLAL")
    // Date code is 7 chars (e.g. 26AUG18)
    if event_part.len() < 13 {
        return Err(MatchError::MalformedTicker(
            ticker.to_string(),
            "event part too short (must contain 7-char date and two 3-char team codes)",
        ));
    }

    let date_code = event_part[..7].to_string();
    let teams_str = &event_part[7..];

    if teams_str.len() != 6 {
        return Err(MatchError::MalformedTicker(
            ticker.to_string(),
            "team codes in event part must be exactly 6 characters (3 away + 3 home)",
        ));
    }

    let away_code = &teams_str[..3];
    let home_code = &teams_str[3..];

    let away_team = lookup_team(away_code, Some(sport))?;
    let home_team = lookup_team(home_code, Some(sport))?;
    let matchup = Matchup::new(home_team, away_team);

    // 3. Parse outcome part if present
    let mut market_kind = None;
    let mut target_team = None;
    let mut is_over = None;

    match kind_hint {
        KalshiKindHint::Game => {
            market_kind = Some(MarketKind::Moneyline);
            if let Some(target) = outcome_part {
                target_team = Some(lookup_team(target, Some(sport))?);
            }
        }
        KalshiKindHint::Spread => {
            if let Some(outcome) = outcome_part {
                let (team, line) = parse_spread_outcome(outcome, sport)?;
                target_team = Some(team);
                // Normalized to home team view:
                // If the target team is home, line is as quoted.
                // If target team is away, spread on home is mirrored.
                let normalized_line = if team == home_team {
                    line
                } else if team == away_team {
                    line.mirrored()
                } else {
                    return Err(MatchError::UnrecognizedTeam(team.code.to_string()));
                };
                market_kind = Some(MarketKind::Spread(normalized_line));
            }
        }
        KalshiKindHint::Total => {
            if let Some(outcome) = outcome_part {
                let (line, over) = parse_total_outcome(outcome)?;
                market_kind = Some(MarketKind::Total(line));
                is_over = Some(over);
            }
        }
        KalshiKindHint::Unknown => {}
    }

    Ok(KalshiTicker {
        sport,
        date_code,
        matchup,
        market_kind,
        target_team,
        is_over,
    })
}

enum KalshiKindHint {
    Game,
    Spread,
    Total,
    Unknown,
}

fn parse_series_prefix(prefix: &str) -> Result<(Sport, KalshiKindHint)> {
    let sport = if prefix.starts_with("KXNBA") {
        Sport::Nba
    } else if prefix.starts_with("KXNFL") {
        Sport::Nfl
    } else if prefix.starts_with("KXMLB") {
        Sport::Mlb
    } else if prefix.starts_with("KXNHL") {
        Sport::Nhl
    } else {
        Sport::Other
    };

    let kind_hint = if prefix.contains("GAME") {
        KalshiKindHint::Game
    } else if prefix.contains("SPREAD") {
        KalshiKindHint::Spread
    } else if prefix.contains("TOTAL") {
        KalshiKindHint::Total
    } else {
        KalshiKindHint::Unknown
    };

    Ok((sport, kind_hint))
}

fn parse_spread_outcome(outcome: &str, sport: Sport) -> Result<(&'static CanonicalTeam, Line)> {
    if outcome.len() < 4 {
        return Err(MatchError::MalformedTicker(
            outcome.to_string(),
            "spread outcome must contain 3-char team code and line digits",
        ));
    }

    let team_code = &outcome[..3];
    let team = lookup_team(team_code, Some(sport))?;
    let line_str = &outcome[3..];

    // e.g. "35" -> 3.5 -> 350 hundredths, or "3.5" -> 350 hundredths, or "-35" -> -350 hundredths
    let hundredths = parse_points_to_hundredths(line_str)?;
    Ok((team, Line::from_hundredths(hundredths)))
}

fn parse_total_outcome(outcome: &str) -> Result<(Line, bool)> {
    if outcome.is_empty() {
        return Err(MatchError::MalformedTicker(
            outcome.to_string(),
            "empty total outcome string",
        ));
    }

    let is_over = if outcome.ends_with('O') || outcome.ends_with('o') {
        true
    } else if outcome.ends_with('U') || outcome.ends_with('u') {
        false
    } else if outcome.starts_with('O') || outcome.starts_with('o') {
        true
    } else if outcome.starts_with('U') || outcome.starts_with('u') {
        false
    } else {
        return Err(MatchError::MalformedTicker(
            outcome.to_string(),
            "total outcome must end or start with 'O' (Over) or 'U' (Under)",
        ));
    };

    let digits_part = outcome
        .trim_matches(|c: char| c == 'O' || c == 'o' || c == 'U' || c == 'u')
        .trim();

    // e.g. "2205" -> 220.5 -> 22050 hundredths
    let hundredths = parse_points_to_hundredths(digits_part)?;
    Ok((Line::from_hundredths(hundredths), is_over))
}

fn parse_points_to_hundredths(s: &str) -> Result<i32> {
    if let Ok(float_val) = s.parse::<f64>() {
        // If integer string without dot like "35" for 3.5 or "2205" for 220.5:
        if !s.contains('.') {
            // If length is 2 (e.g. 35 -> 3.5)
            // Or length is 4 (e.g. 2205 -> 220.5)
            // Standard Kalshi convention: last digit is tenths
            let int_val: i32 = s.parse().map_err(|_| {
                MatchError::MalformedTicker(s.to_string(), "failed to parse point number")
            })?;
            return Ok(int_val * 10);
        }
        let rounded = (float_val * 100.0).round() as i32;
        Ok(rounded)
    } else {
        Err(MatchError::MalformedTicker(
            s.to_string(),
            "invalid line number format",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_kalshi_game_moneyline_ticker() {
        let ticker = "KXNBAGAME-26AUG18BOSLAL-BOS";
        let parsed = parse_kalshi_ticker(ticker).unwrap();

        assert_eq!(parsed.sport, Sport::Nba);
        assert_eq!(parsed.date_code, "26AUG18");
        assert_eq!(parsed.matchup.away.code, "BOS");
        assert_eq!(parsed.matchup.home.code, "LAL");
        assert_eq!(parsed.market_kind, Some(MarketKind::Moneyline));
        assert_eq!(parsed.target_team.unwrap().code, "BOS");
    }

    #[test]
    fn parse_kalshi_spread_ticker() {
        let ticker = "KXNBASPREAD-26AUG18BOSLAL-LAL35";
        let parsed = parse_kalshi_ticker(ticker).unwrap();

        assert_eq!(parsed.sport, Sport::Nba);
        assert_eq!(parsed.matchup.home.code, "LAL");
        assert_eq!(parsed.matchup.away.code, "BOS");
        assert_eq!(parsed.target_team.unwrap().code, "LAL");
        // LAL is home, so LAL +3.5 or 3.5 gives Line(350)
        assert_eq!(
            parsed.market_kind,
            Some(MarketKind::Spread(Line::from_hundredths(350)))
        );
    }

    #[test]
    fn parse_kalshi_total_ticker() {
        let ticker = "KXNBATOTAL-26AUG18BOSLAL-2205O";
        let parsed = parse_kalshi_ticker(ticker).unwrap();

        assert_eq!(parsed.sport, Sport::Nba);
        assert_eq!(parsed.is_over, Some(true));
        assert_eq!(
            parsed.market_kind,
            Some(MarketKind::Total(Line::from_hundredths(22050)))
        );
    }
}
