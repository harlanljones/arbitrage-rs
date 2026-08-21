//! Canonical team identification, aliasing, and matchup parsing.
//!
//! Different venues identify the same sports team with varying names, abbreviations,
//! and city prefixes (e.g., "Boston Celtics", "BOS", "Celtics", "Boston", "BOS Celtics").
//! This module normalizes diverse input strings into canonical, static team representations
//! and parses matchup strings into home and away teams.

use crate::error::{MatchError, Result};

/// A sport or league category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sport {
    /// National Basketball Association.
    Nba,
    /// National Football League.
    Nfl,
    /// Major League Baseball.
    Mlb,
    /// National Hockey League.
    Nhl,
    /// Association Football (Soccer).
    Soccer,
    /// Other sport or unspecified category.
    Other,
}

/// A canonical sports team definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CanonicalTeam {
    /// Sport or league this team belongs to.
    pub sport: Sport,
    /// Standard three-character or short ticker code (e.g. "BOS", "LAL").
    pub code: &'static str,
    /// Full franchise name (e.g. "Boston Celtics", "Los Angeles Lakers").
    pub full_name: &'static str,
    /// Geographic city or region name (e.g. "Boston", "Los Angeles").
    pub city: &'static str,
    /// Team mascot/nickname (e.g. "Celtics", "Lakers").
    pub mascot: &'static str,
}

impl CanonicalTeam {
    /// Create a new canonical team definition.
    pub const fn new(
        sport: Sport,
        code: &'static str,
        full_name: &'static str,
        city: &'static str,
        mascot: &'static str,
    ) -> Self {
        Self {
            sport,
            code,
            full_name,
            city,
            mascot,
        }
    }
}

/// A matchup consisting of a canonical home team and an away team.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Matchup {
    /// The home team hosting the event.
    pub home: &'static CanonicalTeam,
    /// The visiting away team.
    pub away: &'static CanonicalTeam,
}

impl Matchup {
    /// Create a new matchup with specified home and away teams.
    pub const fn new(home: &'static CanonicalTeam, away: &'static CanonicalTeam) -> Self {
        Self { home, away }
    }

    /// Reverse home and away roles.
    pub const fn reversed(self) -> Self {
        Self {
            home: self.away,
            away: self.home,
        }
    }
}

// NBA Teams
const NBA_ATL: CanonicalTeam =
    CanonicalTeam::new(Sport::Nba, "ATL", "Atlanta Hawks", "Atlanta", "Hawks");
const NBA_BOS: CanonicalTeam =
    CanonicalTeam::new(Sport::Nba, "BOS", "Boston Celtics", "Boston", "Celtics");
const NBA_BKN: CanonicalTeam =
    CanonicalTeam::new(Sport::Nba, "BKN", "Brooklyn Nets", "Brooklyn", "Nets");
const NBA_CHA: CanonicalTeam = CanonicalTeam::new(
    Sport::Nba,
    "CHA",
    "Charlotte Hornets",
    "Charlotte",
    "Hornets",
);
const NBA_CHI: CanonicalTeam =
    CanonicalTeam::new(Sport::Nba, "CHI", "Chicago Bulls", "Chicago", "Bulls");
const NBA_CLE: CanonicalTeam = CanonicalTeam::new(
    Sport::Nba,
    "CLE",
    "Cleveland Cavaliers",
    "Cleveland",
    "Cavaliers",
);
const NBA_DAL: CanonicalTeam =
    CanonicalTeam::new(Sport::Nba, "DAL", "Dallas Mavericks", "Dallas", "Mavericks");
const NBA_DEN: CanonicalTeam =
    CanonicalTeam::new(Sport::Nba, "DEN", "Denver Nuggets", "Denver", "Nuggets");
const NBA_DET: CanonicalTeam =
    CanonicalTeam::new(Sport::Nba, "DET", "Detroit Pistons", "Detroit", "Pistons");
const NBA_GSW: CanonicalTeam = CanonicalTeam::new(
    Sport::Nba,
    "GSW",
    "Golden State Warriors",
    "Golden State",
    "Warriors",
);
const NBA_HOU: CanonicalTeam =
    CanonicalTeam::new(Sport::Nba, "HOU", "Houston Rockets", "Houston", "Rockets");
const NBA_IND: CanonicalTeam =
    CanonicalTeam::new(Sport::Nba, "IND", "Indiana Pacers", "Indiana", "Pacers");
const NBA_LAC: CanonicalTeam = CanonicalTeam::new(
    Sport::Nba,
    "LAC",
    "Los Angeles Clippers",
    "Los Angeles",
    "Clippers",
);
const NBA_LAL: CanonicalTeam = CanonicalTeam::new(
    Sport::Nba,
    "LAL",
    "Los Angeles Lakers",
    "Los Angeles",
    "Lakers",
);
const NBA_MEM: CanonicalTeam = CanonicalTeam::new(
    Sport::Nba,
    "MEM",
    "Memphis Grizzlies",
    "Memphis",
    "Grizzlies",
);
const NBA_MIA: CanonicalTeam = CanonicalTeam::new(Sport::Nba, "MIA", "Miami Heat", "Miami", "Heat");
const NBA_MIL: CanonicalTeam =
    CanonicalTeam::new(Sport::Nba, "MIL", "Milwaukee Bucks", "Milwaukee", "Bucks");
const NBA_MIN: CanonicalTeam = CanonicalTeam::new(
    Sport::Nba,
    "MIN",
    "Minnesota Timberwolves",
    "Minnesota",
    "Timberwolves",
);
const NBA_NOP: CanonicalTeam = CanonicalTeam::new(
    Sport::Nba,
    "NOP",
    "New Orleans Pelicans",
    "New Orleans",
    "Pelicans",
);
const NBA_NYK: CanonicalTeam =
    CanonicalTeam::new(Sport::Nba, "NYK", "New York Knicks", "New York", "Knicks");
const NBA_OKC: CanonicalTeam = CanonicalTeam::new(
    Sport::Nba,
    "OKC",
    "Oklahoma City Thunder",
    "Oklahoma City",
    "Thunder",
);
const NBA_ORL: CanonicalTeam =
    CanonicalTeam::new(Sport::Nba, "ORL", "Orlando Magic", "Orlando", "Magic");
const NBA_PHI: CanonicalTeam = CanonicalTeam::new(
    Sport::Nba,
    "PHI",
    "Philadelphia 76ers",
    "Philadelphia",
    "76ers",
);
const NBA_PHX: CanonicalTeam =
    CanonicalTeam::new(Sport::Nba, "PHX", "Phoenix Suns", "Phoenix", "Suns");
const NBA_POR: CanonicalTeam = CanonicalTeam::new(
    Sport::Nba,
    "POR",
    "Portland Trail Blazers",
    "Portland",
    "Trail Blazers",
);
const NBA_SAC: CanonicalTeam =
    CanonicalTeam::new(Sport::Nba, "SAC", "Sacramento Kings", "Sacramento", "Kings");
const NBA_SAS: CanonicalTeam = CanonicalTeam::new(
    Sport::Nba,
    "SAS",
    "San Antonio Spurs",
    "San Antonio",
    "Spurs",
);
const NBA_TOR: CanonicalTeam =
    CanonicalTeam::new(Sport::Nba, "TOR", "Toronto Raptors", "Toronto", "Raptors");
const NBA_UTA: CanonicalTeam = CanonicalTeam::new(Sport::Nba, "UTA", "Utah Jazz", "Utah", "Jazz");
const NBA_WAS: CanonicalTeam = CanonicalTeam::new(
    Sport::Nba,
    "WAS",
    "Washington Wizards",
    "Washington",
    "Wizards",
);

// NFL Teams (Selected common representatives)
const NFL_KC: CanonicalTeam = CanonicalTeam::new(
    Sport::Nfl,
    "KC",
    "Kansas City Chiefs",
    "Kansas City",
    "Chiefs",
);
const NFL_SF: CanonicalTeam = CanonicalTeam::new(
    Sport::Nfl,
    "SF",
    "San Francisco 49ers",
    "San Francisco",
    "49ers",
);
const NFL_BAL: CanonicalTeam =
    CanonicalTeam::new(Sport::Nfl, "BAL", "Baltimore Ravens", "Baltimore", "Ravens");
const NFL_BUF: CanonicalTeam =
    CanonicalTeam::new(Sport::Nfl, "BUF", "Buffalo Bills", "Buffalo", "Bills");
const NFL_DAL: CanonicalTeam =
    CanonicalTeam::new(Sport::Nfl, "DAL", "Dallas Cowboys", "Dallas", "Cowboys");
const NFL_GB: CanonicalTeam = CanonicalTeam::new(
    Sport::Nfl,
    "GB",
    "Green Bay Packers",
    "Green Bay",
    "Packers",
);
const NFL_NE: CanonicalTeam = CanonicalTeam::new(
    Sport::Nfl,
    "NE",
    "New England Patriots",
    "New England",
    "Patriots",
);
const NFL_PHI: CanonicalTeam = CanonicalTeam::new(
    Sport::Nfl,
    "PHI",
    "Philadelphia Eagles",
    "Philadelphia",
    "Eagles",
);

// MLB Teams (Selected common representatives)
const MLB_BOS: CanonicalTeam =
    CanonicalTeam::new(Sport::Mlb, "BOS", "Boston Red Sox", "Boston", "Red Sox");
const MLB_NYY: CanonicalTeam =
    CanonicalTeam::new(Sport::Mlb, "NYY", "New York Yankees", "New York", "Yankees");
const MLB_LAD: CanonicalTeam = CanonicalTeam::new(
    Sport::Mlb,
    "LAD",
    "Los Angeles Dodgers",
    "Los Angeles",
    "Dodgers",
);

// NHL Teams (Selected common representatives)
const NHL_BOS: CanonicalTeam =
    CanonicalTeam::new(Sport::Nhl, "BOS", "Boston Bruins", "Boston", "Bruins");
const NHL_TOR: CanonicalTeam = CanonicalTeam::new(
    Sport::Nhl,
    "TOR",
    "Toronto Maple Leafs",
    "Toronto",
    "Maple Leafs",
);
const NHL_EDM: CanonicalTeam =
    CanonicalTeam::new(Sport::Nhl, "EDM", "Edmonton Oilers", "Edmonton", "Oilers");

struct AliasMapping {
    alias: &'static str,
    team: &'static CanonicalTeam,
}

// Pre-compiled aliases table
static ALIASES: &[AliasMapping] = &[
    // NBA
    AliasMapping {
        alias: "atl",
        team: &NBA_ATL,
    },
    AliasMapping {
        alias: "atlanta",
        team: &NBA_ATL,
    },
    AliasMapping {
        alias: "hawks",
        team: &NBA_ATL,
    },
    AliasMapping {
        alias: "atlantahawks",
        team: &NBA_ATL,
    },
    AliasMapping {
        alias: "bos",
        team: &NBA_BOS,
    },
    AliasMapping {
        alias: "boston",
        team: &NBA_BOS,
    },
    AliasMapping {
        alias: "celtics",
        team: &NBA_BOS,
    },
    AliasMapping {
        alias: "bostonceltics",
        team: &NBA_BOS,
    },
    AliasMapping {
        alias: "bosceltics",
        team: &NBA_BOS,
    },
    AliasMapping {
        alias: "bkn",
        team: &NBA_BKN,
    },
    AliasMapping {
        alias: "brooklyn",
        team: &NBA_BKN,
    },
    AliasMapping {
        alias: "nets",
        team: &NBA_BKN,
    },
    AliasMapping {
        alias: "brooklynnets",
        team: &NBA_BKN,
    },
    AliasMapping {
        alias: "bknnets",
        team: &NBA_BKN,
    },
    AliasMapping {
        alias: "cha",
        team: &NBA_CHA,
    },
    AliasMapping {
        alias: "charlotte",
        team: &NBA_CHA,
    },
    AliasMapping {
        alias: "hornets",
        team: &NBA_CHA,
    },
    AliasMapping {
        alias: "charlottehornets",
        team: &NBA_CHA,
    },
    AliasMapping {
        alias: "chi",
        team: &NBA_CHI,
    },
    AliasMapping {
        alias: "chicago",
        team: &NBA_CHI,
    },
    AliasMapping {
        alias: "bulls",
        team: &NBA_CHI,
    },
    AliasMapping {
        alias: "chicagobulls",
        team: &NBA_CHI,
    },
    AliasMapping {
        alias: "cle",
        team: &NBA_CLE,
    },
    AliasMapping {
        alias: "cleveland",
        team: &NBA_CLE,
    },
    AliasMapping {
        alias: "cavaliers",
        team: &NBA_CLE,
    },
    AliasMapping {
        alias: "cavs",
        team: &NBA_CLE,
    },
    AliasMapping {
        alias: "clevelandcavaliers",
        team: &NBA_CLE,
    },
    AliasMapping {
        alias: "dal",
        team: &NBA_DAL,
    },
    AliasMapping {
        alias: "dallas",
        team: &NBA_DAL,
    },
    AliasMapping {
        alias: "mavericks",
        team: &NBA_DAL,
    },
    AliasMapping {
        alias: "mavs",
        team: &NBA_DAL,
    },
    AliasMapping {
        alias: "dallasmavericks",
        team: &NBA_DAL,
    },
    AliasMapping {
        alias: "den",
        team: &NBA_DEN,
    },
    AliasMapping {
        alias: "denver",
        team: &NBA_DEN,
    },
    AliasMapping {
        alias: "nuggets",
        team: &NBA_DEN,
    },
    AliasMapping {
        alias: "denvernuggets",
        team: &NBA_DEN,
    },
    AliasMapping {
        alias: "det",
        team: &NBA_DET,
    },
    AliasMapping {
        alias: "detroit",
        team: &NBA_DET,
    },
    AliasMapping {
        alias: "pistons",
        team: &NBA_DET,
    },
    AliasMapping {
        alias: "detroitpistons",
        team: &NBA_DET,
    },
    AliasMapping {
        alias: "gsw",
        team: &NBA_GSW,
    },
    AliasMapping {
        alias: "gs",
        team: &NBA_GSW,
    },
    AliasMapping {
        alias: "goldenstate",
        team: &NBA_GSW,
    },
    AliasMapping {
        alias: "warriors",
        team: &NBA_GSW,
    },
    AliasMapping {
        alias: "goldenstatewarriors",
        team: &NBA_GSW,
    },
    AliasMapping {
        alias: "gswarriors",
        team: &NBA_GSW,
    },
    AliasMapping {
        alias: "hou",
        team: &NBA_HOU,
    },
    AliasMapping {
        alias: "houston",
        team: &NBA_HOU,
    },
    AliasMapping {
        alias: "rockets",
        team: &NBA_HOU,
    },
    AliasMapping {
        alias: "houstonrockets",
        team: &NBA_HOU,
    },
    AliasMapping {
        alias: "ind",
        team: &NBA_IND,
    },
    AliasMapping {
        alias: "indiana",
        team: &NBA_IND,
    },
    AliasMapping {
        alias: "pacers",
        team: &NBA_IND,
    },
    AliasMapping {
        alias: "indianapacers",
        team: &NBA_IND,
    },
    AliasMapping {
        alias: "lac",
        team: &NBA_LAC,
    },
    AliasMapping {
        alias: "clippers",
        team: &NBA_LAC,
    },
    AliasMapping {
        alias: "laclippers",
        team: &NBA_LAC,
    },
    AliasMapping {
        alias: "losangelesclippers",
        team: &NBA_LAC,
    },
    AliasMapping {
        alias: "lal",
        team: &NBA_LAL,
    },
    AliasMapping {
        alias: "lakers",
        team: &NBA_LAL,
    },
    AliasMapping {
        alias: "lalakers",
        team: &NBA_LAL,
    },
    AliasMapping {
        alias: "losangeleslakers",
        team: &NBA_LAL,
    },
    AliasMapping {
        alias: "mem",
        team: &NBA_MEM,
    },
    AliasMapping {
        alias: "memphis",
        team: &NBA_MEM,
    },
    AliasMapping {
        alias: "grizzlies",
        team: &NBA_MEM,
    },
    AliasMapping {
        alias: "memphisgrizzlies",
        team: &NBA_MEM,
    },
    AliasMapping {
        alias: "mia",
        team: &NBA_MIA,
    },
    AliasMapping {
        alias: "miami",
        team: &NBA_MIA,
    },
    AliasMapping {
        alias: "heat",
        team: &NBA_MIA,
    },
    AliasMapping {
        alias: "miamiheat",
        team: &NBA_MIA,
    },
    AliasMapping {
        alias: "mil",
        team: &NBA_MIL,
    },
    AliasMapping {
        alias: "milwaukee",
        team: &NBA_MIL,
    },
    AliasMapping {
        alias: "bucks",
        team: &NBA_MIL,
    },
    AliasMapping {
        alias: "milwaukeebucks",
        team: &NBA_MIL,
    },
    AliasMapping {
        alias: "min",
        team: &NBA_MIN,
    },
    AliasMapping {
        alias: "minnesota",
        team: &NBA_MIN,
    },
    AliasMapping {
        alias: "timberwolves",
        team: &NBA_MIN,
    },
    AliasMapping {
        alias: "wolves",
        team: &NBA_MIN,
    },
    AliasMapping {
        alias: "minnesotatimberwolves",
        team: &NBA_MIN,
    },
    AliasMapping {
        alias: "nop",
        team: &NBA_NOP,
    },
    AliasMapping {
        alias: "no",
        team: &NBA_NOP,
    },
    AliasMapping {
        alias: "neworleans",
        team: &NBA_NOP,
    },
    AliasMapping {
        alias: "pelicans",
        team: &NBA_NOP,
    },
    AliasMapping {
        alias: "neworleanspelicans",
        team: &NBA_NOP,
    },
    AliasMapping {
        alias: "nyk",
        team: &NBA_NYK,
    },
    AliasMapping {
        alias: "ny",
        team: &NBA_NYK,
    },
    AliasMapping {
        alias: "knicks",
        team: &NBA_NYK,
    },
    AliasMapping {
        alias: "newyorkknicks",
        team: &NBA_NYK,
    },
    AliasMapping {
        alias: "nyknicks",
        team: &NBA_NYK,
    },
    AliasMapping {
        alias: "okc",
        team: &NBA_OKC,
    },
    AliasMapping {
        alias: "oklahomacity",
        team: &NBA_OKC,
    },
    AliasMapping {
        alias: "thunder",
        team: &NBA_OKC,
    },
    AliasMapping {
        alias: "oklahomacitythunder",
        team: &NBA_OKC,
    },
    AliasMapping {
        alias: "okcthunder",
        team: &NBA_OKC,
    },
    AliasMapping {
        alias: "orl",
        team: &NBA_ORL,
    },
    AliasMapping {
        alias: "orlando",
        team: &NBA_ORL,
    },
    AliasMapping {
        alias: "magic",
        team: &NBA_ORL,
    },
    AliasMapping {
        alias: "orlandomagic",
        team: &NBA_ORL,
    },
    AliasMapping {
        alias: "phi",
        team: &NBA_PHI,
    },
    AliasMapping {
        alias: "philadelphia",
        team: &NBA_PHI,
    },
    AliasMapping {
        alias: "76ers",
        team: &NBA_PHI,
    },
    AliasMapping {
        alias: "sixers",
        team: &NBA_PHI,
    },
    AliasMapping {
        alias: "philadelphia76ers",
        team: &NBA_PHI,
    },
    AliasMapping {
        alias: "phx",
        team: &NBA_PHX,
    },
    AliasMapping {
        alias: "phoenix",
        team: &NBA_PHX,
    },
    AliasMapping {
        alias: "suns",
        team: &NBA_PHX,
    },
    AliasMapping {
        alias: "phoenixsuns",
        team: &NBA_PHX,
    },
    AliasMapping {
        alias: "por",
        team: &NBA_POR,
    },
    AliasMapping {
        alias: "portland",
        team: &NBA_POR,
    },
    AliasMapping {
        alias: "trailblazers",
        team: &NBA_POR,
    },
    AliasMapping {
        alias: "blazers",
        team: &NBA_POR,
    },
    AliasMapping {
        alias: "portlandtrailblazers",
        team: &NBA_POR,
    },
    AliasMapping {
        alias: "sac",
        team: &NBA_SAC,
    },
    AliasMapping {
        alias: "sacramento",
        team: &NBA_SAC,
    },
    AliasMapping {
        alias: "kings",
        team: &NBA_SAC,
    },
    AliasMapping {
        alias: "sacramentokings",
        team: &NBA_SAC,
    },
    AliasMapping {
        alias: "sas",
        team: &NBA_SAS,
    },
    AliasMapping {
        alias: "sa",
        team: &NBA_SAS,
    },
    AliasMapping {
        alias: "sanantonio",
        team: &NBA_SAS,
    },
    AliasMapping {
        alias: "spurs",
        team: &NBA_SAS,
    },
    AliasMapping {
        alias: "sanantoniospurs",
        team: &NBA_SAS,
    },
    AliasMapping {
        alias: "tor",
        team: &NBA_TOR,
    },
    AliasMapping {
        alias: "toronto",
        team: &NBA_TOR,
    },
    AliasMapping {
        alias: "raptors",
        team: &NBA_TOR,
    },
    AliasMapping {
        alias: "torontoraptors",
        team: &NBA_TOR,
    },
    AliasMapping {
        alias: "uta",
        team: &NBA_UTA,
    },
    AliasMapping {
        alias: "utah",
        team: &NBA_UTA,
    },
    AliasMapping {
        alias: "jazz",
        team: &NBA_UTA,
    },
    AliasMapping {
        alias: "utahjazz",
        team: &NBA_UTA,
    },
    AliasMapping {
        alias: "was",
        team: &NBA_WAS,
    },
    AliasMapping {
        alias: "washington",
        team: &NBA_WAS,
    },
    AliasMapping {
        alias: "wizards",
        team: &NBA_WAS,
    },
    AliasMapping {
        alias: "washingtonwizards",
        team: &NBA_WAS,
    },
    // NFL Aliases
    AliasMapping {
        alias: "kc",
        team: &NFL_KC,
    },
    AliasMapping {
        alias: "kansascity",
        team: &NFL_KC,
    },
    AliasMapping {
        alias: "chiefs",
        team: &NFL_KC,
    },
    AliasMapping {
        alias: "kansascitychiefs",
        team: &NFL_KC,
    },
    AliasMapping {
        alias: "sf",
        team: &NFL_SF,
    },
    AliasMapping {
        alias: "sanfrancisco",
        team: &NFL_SF,
    },
    AliasMapping {
        alias: "49ers",
        team: &NFL_SF,
    },
    AliasMapping {
        alias: "niners",
        team: &NFL_SF,
    },
    AliasMapping {
        alias: "sanfrancisco49ers",
        team: &NFL_SF,
    },
    AliasMapping {
        alias: "bal",
        team: &NFL_BAL,
    },
    AliasMapping {
        alias: "baltimore",
        team: &NFL_BAL,
    },
    AliasMapping {
        alias: "ravens",
        team: &NFL_BAL,
    },
    AliasMapping {
        alias: "baltimoreravens",
        team: &NFL_BAL,
    },
    AliasMapping {
        alias: "buf",
        team: &NFL_BUF,
    },
    AliasMapping {
        alias: "buffalo",
        team: &NFL_BUF,
    },
    AliasMapping {
        alias: "bills",
        team: &NFL_BUF,
    },
    AliasMapping {
        alias: "buffalobills",
        team: &NFL_BUF,
    },
    AliasMapping {
        alias: "cowboys",
        team: &NFL_DAL,
    },
    AliasMapping {
        alias: "dallascowboys",
        team: &NFL_DAL,
    },
    AliasMapping {
        alias: "gb",
        team: &NFL_GB,
    },
    AliasMapping {
        alias: "greenbay",
        team: &NFL_GB,
    },
    AliasMapping {
        alias: "packers",
        team: &NFL_GB,
    },
    AliasMapping {
        alias: "greenbaypackers",
        team: &NFL_GB,
    },
    AliasMapping {
        alias: "ne",
        team: &NFL_NE,
    },
    AliasMapping {
        alias: "newengland",
        team: &NFL_NE,
    },
    AliasMapping {
        alias: "patriots",
        team: &NFL_NE,
    },
    AliasMapping {
        alias: "pats",
        team: &NFL_NE,
    },
    AliasMapping {
        alias: "newenglandpatriots",
        team: &NFL_NE,
    },
    AliasMapping {
        alias: "eagles",
        team: &NFL_PHI,
    },
    AliasMapping {
        alias: "philadelphiaeagles",
        team: &NFL_PHI,
    },
    // MLB Aliases
    AliasMapping {
        alias: "bos",
        team: &MLB_BOS,
    },
    AliasMapping {
        alias: "boston",
        team: &MLB_BOS,
    },
    AliasMapping {
        alias: "redsox",
        team: &MLB_BOS,
    },
    AliasMapping {
        alias: "bostonredsox",
        team: &MLB_BOS,
    },
    AliasMapping {
        alias: "nyy",
        team: &MLB_NYY,
    },
    AliasMapping {
        alias: "yankees",
        team: &MLB_NYY,
    },
    AliasMapping {
        alias: "newyorkyankees",
        team: &MLB_NYY,
    },
    AliasMapping {
        alias: "lad",
        team: &MLB_LAD,
    },
    AliasMapping {
        alias: "dodgers",
        team: &MLB_LAD,
    },
    AliasMapping {
        alias: "losangelesdodgers",
        team: &MLB_LAD,
    },
    AliasMapping {
        alias: "ladodgers",
        team: &MLB_LAD,
    },
    // NHL Aliases
    AliasMapping {
        alias: "bos",
        team: &NHL_BOS,
    },
    AliasMapping {
        alias: "boston",
        team: &NHL_BOS,
    },
    AliasMapping {
        alias: "bruins",
        team: &NHL_BOS,
    },
    AliasMapping {
        alias: "bostonbruins",
        team: &NHL_BOS,
    },
    AliasMapping {
        alias: "mapleleafs",
        team: &NHL_TOR,
    },
    AliasMapping {
        alias: "leafs",
        team: &NHL_TOR,
    },
    AliasMapping {
        alias: "torontomapleleafs",
        team: &NHL_TOR,
    },
    AliasMapping {
        alias: "edm",
        team: &NHL_EDM,
    },
    AliasMapping {
        alias: "edmonton",
        team: &NHL_EDM,
    },
    AliasMapping {
        alias: "oilers",
        team: &NHL_EDM,
    },
    AliasMapping {
        alias: "edmontonoilers",
        team: &NHL_EDM,
    },
];

/// Normalize a raw string for alias dictionary lookup.
///
/// Strips punctuation, parentheses, brackets, and whitespace, returning a lowercase alphanumeric string.
pub fn normalize_string(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Lookup a canonical team from a raw name, abbreviation, or alias.
///
/// If `sport_hint` is provided, candidates matching the specified sport will be preferred.
pub fn lookup_team(raw: &str, sport_hint: Option<Sport>) -> Result<&'static CanonicalTeam> {
    let normalized = normalize_string(raw);
    if normalized.is_empty() {
        return Err(MatchError::UnrecognizedTeam(raw.to_string()));
    }

    let mut best_match: Option<&'static CanonicalTeam> = None;

    for mapping in ALIASES {
        if mapping.alias == normalized {
            if let Some(sport) = sport_hint {
                if mapping.team.sport == sport {
                    return Ok(mapping.team);
                }
            }
            if best_match.is_none() {
                best_match = Some(mapping.team);
            }
        }
    }

    if let Some(team) = best_match {
        return Ok(team);
    }

    // Secondary pass: prefix or suffix exact match if applicable
    for mapping in ALIASES {
        if mapping.team.code.eq_ignore_ascii_case(&normalized) {
            if let Some(sport) = sport_hint {
                if mapping.team.sport == sport {
                    return Ok(mapping.team);
                }
            }
            if best_match.is_none() {
                best_match = Some(mapping.team);
            }
        }
    }

    best_match.ok_or_else(|| MatchError::UnrecognizedTeam(raw.to_string()))
}

/// Parse a raw matchup string into canonical home and away teams.
///
/// Supports common patterns:
/// - `"BOS @ LAL"` or `"Boston Celtics at Los Angeles Lakers"` (First is away, second is home)
/// - `"Los Angeles Lakers vs Boston Celtics"` or `"LAL vs. BOS"` or `"LAL v BOS"` (First is home, second is away)
pub fn parse_matchup(raw: &str, sport_hint: Option<Sport>) -> Result<Matchup> {
    let trimmed = raw.trim();

    // Check for "@" or " at " indicating Away @ Home
    if let Some((away_part, home_part)) = split_matchup(trimmed, &[" @ ", "@", " at "]) {
        let away = lookup_team(away_part, sport_hint)?;
        let home = lookup_team(home_part, sport_hint)?;
        return Ok(Matchup::new(home, away));
    }

    // Check for "vs", "vs.", "v", "v." indicating Home vs Away
    if let Some((home_part, away_part)) =
        split_matchup(trimmed, &[" vs. ", " vs ", " v. ", " v ", " - "])
    {
        let home = lookup_team(home_part, sport_hint)?;
        let away = lookup_team(away_part, sport_hint)?;
        return Ok(Matchup::new(home, away));
    }

    Err(MatchError::MalformedMatchup(raw.to_string()))
}

fn split_matchup<'a>(input: &'a str, delimiters: &[&str]) -> Option<(&'a str, &'a str)> {
    for &delim in delimiters {
        if let Some(idx) = input.find(delim) {
            let left = input[..idx].trim();
            let right = input[idx + delim.len()..].trim();
            if !left.is_empty() && !right.is_empty() {
                return Some((left, right));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn team_name_normalization_resolves_common_aliases() {
        let bos = lookup_team("Boston Celtics", Some(Sport::Nba)).unwrap();
        assert_eq!(bos.code, "BOS");

        let bos2 = lookup_team("BOS", Some(Sport::Nba)).unwrap();
        assert_eq!(bos2.code, "BOS");

        let bos3 = lookup_team("Celtics", Some(Sport::Nba)).unwrap();
        assert_eq!(bos3.code, "BOS");

        let lal = lookup_team("LA Lakers", Some(Sport::Nba)).unwrap();
        assert_eq!(lal.code, "LAL");

        let gsw = lookup_team("GS Warriors", Some(Sport::Nba)).unwrap();
        assert_eq!(gsw.code, "GSW");

        let phi = lookup_team("76ers", Some(Sport::Nba)).unwrap();
        assert_eq!(phi.code, "PHI");
    }

    #[test]
    fn sport_hints_disambiguate_shared_cities() {
        let nba_bos = lookup_team("Boston", Some(Sport::Nba)).unwrap();
        assert_eq!(nba_bos.sport, Sport::Nba);
        assert_eq!(nba_bos.code, "BOS");
        assert_eq!(nba_bos.mascot, "Celtics");

        let mlb_bos = lookup_team("Boston", Some(Sport::Mlb)).unwrap();
        assert_eq!(mlb_bos.sport, Sport::Mlb);
        assert_eq!(mlb_bos.code, "BOS");
        assert_eq!(mlb_bos.mascot, "Red Sox");

        let nhl_bos = lookup_team("Boston", Some(Sport::Nhl)).unwrap();
        assert_eq!(nhl_bos.sport, Sport::Nhl);
        assert_eq!(nhl_bos.code, "BOS");
        assert_eq!(nhl_bos.mascot, "Bruins");
    }

    #[test]
    fn matchup_parsing_at_symbol() {
        let matchup = parse_matchup("BOS @ LAL", Some(Sport::Nba)).unwrap();
        assert_eq!(matchup.away.code, "BOS");
        assert_eq!(matchup.home.code, "LAL");

        let matchup2 =
            parse_matchup("Boston Celtics at Los Angeles Lakers", Some(Sport::Nba)).unwrap();
        assert_eq!(matchup2.away.code, "BOS");
        assert_eq!(matchup2.home.code, "LAL");
    }

    #[test]
    fn matchup_parsing_vs_symbol() {
        let matchup = parse_matchup("LAL vs. BOS", Some(Sport::Nba)).unwrap();
        assert_eq!(matchup.home.code, "LAL");
        assert_eq!(matchup.away.code, "BOS");

        let matchup2 =
            parse_matchup("Los Angeles Lakers vs Boston Celtics", Some(Sport::Nba)).unwrap();
        assert_eq!(matchup2.home.code, "LAL");
        assert_eq!(matchup2.away.code, "BOS");
    }

    #[test]
    fn unknown_teams_or_malformed_matchups_return_errors() {
        assert!(matches!(
            lookup_team("Atlantis Atlantians", None),
            Err(MatchError::UnrecognizedTeam(_))
        ));

        assert!(matches!(
            parse_matchup("JustATeamWithoutSeparator", None),
            Err(MatchError::MalformedMatchup(_))
        ));
    }
}
