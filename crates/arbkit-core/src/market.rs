//! What is being bet on.
//!
//! The types here exist to make one class of catastrophe unrepresentable:
//! pairing two quotes that are not actually opposite sides of the same claim.
//! Getting an odds conversion wrong costs basis points. Hedging Lakers -3.5
//! against Celtics +3.0 costs the whole stake, and it looks like a healthy arb
//! right up until the game lands on 3.

/// A handicap or total, in hundredths of a point.
///
/// Sportsbooks price to the half point almost everywhere and to the quarter
/// point on soccer and tennis handicaps, so hundredths cover every line in
/// circulation with room to spare — and, being integers, two lines are equal
/// when their representations are equal, with no epsilon to tune.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Line(i32);

impl Line {
    /// Build from hundredths of a point: `-350` is `-3.5`.
    #[inline]
    pub const fn from_hundredths(hundredths: i32) -> Line {
        Line(hundredths)
    }

    /// The underlying hundredths.
    #[inline]
    pub const fn hundredths(self) -> i32 {
        self.0
    }

    /// The same line seen from the other side of the bet.
    ///
    /// A spread of -3.5 for one team is +3.5 for the other. A total is *not*
    /// mirrored this way — over 220.5 and under 220.5 share the same line —
    /// which is why this is a method on `Line` and not on `MarketKind`.
    #[inline]
    pub const fn mirrored(self) -> Line {
        Line(-self.0)
    }

    /// Whether this line can push, returning stakes instead of settling.
    ///
    /// A whole-point line can land exactly on the number. That turns a
    /// supposedly risk-free pair into a bet on the other leg, so the engine
    /// must know about it rather than discovering it on settlement day.
    #[inline]
    pub const fn can_push(self) -> bool {
        self.0 % 100 == 0
    }

    /// This line as a float, for display and reporting.
    #[inline]
    pub fn as_f64(self) -> f64 {
        f64::from(self.0) / 100.0
    }
}

/// The kind of market a price belongs to.
///
/// Two quotes are comparable only when their `MarketKind`s are equal, which is
/// derived rather than eyeballed. Spread and total carry their line, so a
/// mismatch cannot compare equal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MarketKind {
    /// Who wins outright. Two outcomes, or three where a draw is possible.
    Moneyline,

    /// Who wins after a handicap is applied, stored from the home side's view.
    ///
    /// Normalizing to one side at construction is what makes the derived
    /// equality trustworthy: Lakers -3.5 and Celtics +3.5 must produce the
    /// same value, not two values a matcher has to reconcile later.
    Spread(Line),

    /// Whether the combined score lands over or under the line.
    Total(Line),
}

impl MarketKind {
    /// Whether a settled result can push, returning stakes.
    #[inline]
    pub const fn can_push(self) -> bool {
        match self {
            MarketKind::Moneyline => false,
            MarketKind::Spread(line) | MarketKind::Total(line) => line.can_push(),
        }
    }
}
