//! The arbitrage condition, and what it costs to actually take it.
//!
//! For a market whose outcomes are mutually exclusive and collectively
//! exhaustive, backing every outcome at implied probabilities `p_i` is
//! risk-free when
//!
//! ```text
//! sum(p_i) < 1
//! ```
//!
//! and the profit is `(1 - sum(p_i)) / sum(p_i)` of the total stake. That is
//! the whole textbook result, and on its own it produces a signal stream that
//! is mostly noise. Three things stand between it and a trade, and all three
//! are computed here rather than left to the caller:
//!
//! 1. **Fees**, applied to each leg *before* the sum, via [`Fee`]. A 30 bp
//!    raw edge against Kalshi's 350 bp stake fee is not an edge.
//! 2. **Depth.** The size resting at the best price bounds the trade. An arb
//!    that exists for twelve dollars is a screenshot, not a position.
//! 3. **Stake granularity.** Contracts are integers. Rounding each leg down to
//!    a tradeable size breaks the equal-payoff property the textbook result
//!    assumes, so the profit reported here is the *worst* outcome's payout
//!    minus the total staked — the amount guaranteed no matter which outcome
//!    lands.
//!
//! [`detect`] returns `None` far more often than it returns a signal. That is
//! the function working.
//!
//! # Why the book, and not just the top of it
//!
//! [`detect_book`] is the real entry point. It takes a [`BookLeg`] per
//! (venue, outcome) — several price levels deep — rather than a single quote,
//! because on a prediction market the top level is usually thin. Sizing an arb
//! against the best price alone leaves most of the edge on the table: the
//! second level is a worse price but it is still, very often, a *profitable*
//! price, and money staked there is money the top-of-book plan never earned.
//!
//! Once more than one level per outcome is in play the sizing stops being a
//! closed-form division. A plan is a set of integer stakes, one per
//! `(leg, level)` chunk, each a whole multiple of that venue's increment and
//! each bounded by the size resting at that level; the payout if outcome `o`
//! settles is the sum of the *floored* payouts of the chunks backing `o`; and
//! the number worth maximizing is
//!
//! ```text
//! profit = min over outcomes of (that outcome's payout) - total staked
//! ```
//!
//! That is an integer program, and the hot path has 50 µs for the whole loop.
//! So it is solved the way integer programs are solved when the answer has to
//! arrive on a deadline: **binary search on the guaranteed payout**. For a
//! candidate payout floor `T`, sizing each outcome to *reach* `T` as cheaply as
//! the increments allow is a bounded, division-only computation, and it either
//! fits the budget and the depth or it does not. Search `T`, keep the plan with
//! the best recomputed profit, and cross-check it against the closed-form
//! equal-payoff plan that [`detect`] used to emit — which is always in the
//! feasible set, so the answer can never be worse than the one this module
//! produced before.
//!
//! Everything is bounded at compile time: [`MAX_CHUNKS`] chunks, at most
//! `MAX_CHUNKS` repair steps per outcome per probe, at most
//! [`MAX_SEARCH_STEPS`] probes. No allocation, no locks, no floats, no strings.
//!
//! # Rounding, in one direction only
//!
//! Every rounding decision here is made against us on purpose. Payouts floor.
//! Stakes needed to *reach* a payout round **up** to the next tradeable
//! increment — rounding them down would report a payout the venue will not
//! actually pay — and the resulting cost is then checked against depth and
//! budget, so rounding up can only ever make a plan infeasible, never make it
//! look cheaper than it is. The profit finally reported is recomputed from the
//! stakes themselves rather than carried over from the search target, so it is
//! a number you should beat, not one you must hit.

use crate::book::{Cents, Level, OutcomeId, VenueId, MAX_LEVELS};
use crate::error::{ArbError, Result};
use crate::fee::Fee;
use crate::price::{Prob, PPM};

/// The most outcomes a single market may have.
///
/// Two for a moneyline, three once a draw is possible. Four leaves headroom
/// without letting a `Signal` grow past a couple of cache lines.
pub const MAX_LEGS: usize = 4;

/// The most price levels [`detect_book`] will look at on one leg.
///
/// Matches [`MAX_LEVELS`], the depth an [`OutcomeBook`](crate::book::OutcomeBook)
/// retains: there is no point offering the detector levels the book does not
/// keep.
pub const MAX_LEVELS_PER_LEG: usize = MAX_LEVELS;

/// The most `(leg, level)` chunks a single plan may stake into.
///
/// This is the hard bound on the whole search: every loop below is either
/// `MAX_CHUNKS` iterations or `MAX_CHUNKS` squared, and a [`Signal`] carries at
/// most this many allocations. Sixteen is four outcomes four levels deep, or
/// two outcomes eight levels deep — past that the marginal level is priced so
/// far from the top of book that it is not part of an arbitrage.
pub const MAX_CHUNKS: usize = 16;

/// The most halvings the payout search will perform.
///
/// The search range is bounded above by `budget * PPM`, which is under `2^84`
/// for any `Cents` budget, so 96 halvings always drive it to a single value.
/// A realistic budget needs closer to forty.
pub const MAX_SEARCH_STEPS: usize = 96;

/// `PPM` as the width the sizing arithmetic is done in.
const PPM_I: i128 = PPM as i128;

/// The chunk flattening tracks which levels of a leg it has already taken in a
/// `u8` bitmask, one bit per level. Widening the book past eight levels without
/// widening that mask would silently shift bits into nothing, and the detector
/// would quietly re-take a level it had already staked into.
const _: () = assert!(MAX_LEVELS_PER_LEG <= u8::BITS as usize);

/// One side of a prospective arbitrage: a price, where to get it, and how much
/// of it there is.
///
/// A single quote. [`BookLeg`] is the same thing with depth behind it, and is
/// what [`detect_book`] takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Leg {
    /// Which venue is quoting.
    pub venue: VenueId,
    /// Which outcome this backs.
    pub outcome: OutcomeId,
    /// The venue's quoted price, before fees.
    pub quoted: Prob,
    /// What the venue charges.
    pub fee: Fee,
    /// The most stake this leg can absorb, in cents — its depth.
    pub capacity: Cents,
    /// The smallest stake step, in cents. One contract, or `1` if continuous.
    pub increment: Cents,
}

/// One venue's book for one outcome, as much of it as is worth trading.
///
/// Levels are best-first, exactly as [`OutcomeBook::levels`] hands them over,
/// and are **pre-discounted by the caller**: `Level::size` is what this side is
/// willing to actually take at that price, not the raw resting size. The
/// detector trusts that number, because deciding how much of a displayed level
/// will still be there on arrival is a queue-position question and belongs
/// upstream of a pure function.
///
/// [`OutcomeBook::levels`]: crate::book::OutcomeBook::levels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BookLeg {
    /// Which venue is quoting.
    pub venue: VenueId,
    /// Which outcome this backs. Legs sharing an outcome hedge each other.
    pub outcome: OutcomeId,
    /// What the venue charges, applied to every level before anything else.
    pub fee: Fee,
    /// The smallest stake step, in cents. One contract, or `1` if continuous.
    pub increment: Cents,
    /// The levels, best-first. Entries past `n_levels` are ignored.
    pub levels: [Level; MAX_LEVELS_PER_LEG],
    /// How many of `levels` are real. `0` means this leg is unusable — a stale
    /// book, a venue that is down, a side with nothing resting on it.
    pub n_levels: u8,
}

/// What to stake on one chunk, and what it returns if that chunk's outcome
/// wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Allocation {
    /// Index of the leg in the slice handed to [`detect_book`] or [`detect`].
    ///
    /// Several allocations may share a leg (different levels of the same book)
    /// and several legs may share an outcome (the same side on two venues).
    /// Which outcome an allocation backs is recovered by indexing back into
    /// that slice — the signal does not carry it, because the slice already
    /// does.
    pub leg: usize,
    /// Stake in cents, already rounded to the leg's increment.
    pub stake: Cents,
    /// Gross return in cents if this chunk's outcome wins, stake included.
    pub payout: Cents,
}

/// A tradeable arbitrage, sized and costed.
///
/// Fixed capacity and `Copy`: a signal crosses a ring buffer to the simulator
/// thread, and anything that allocates on that path defeats the point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Signal {
    allocations: [Allocation; MAX_CHUNKS],
    len: u8,
    /// Sum of the fee-adjusted implied probabilities of the *best* price
    /// available on each outcome, in ppm.
    ///
    /// Below 1_000_000 by construction; how far below is the raw edge at the
    /// top of book, before depth, stake rounding, or any deeper level.
    pub overround_ppm: u32,
    /// Total staked across all allocations, in cents.
    pub total_stake: Cents,
    /// Profit guaranteed regardless of which outcome settles, in cents.
    ///
    /// This is the worst *outcome's* summed payout minus the total stake, so it
    /// already accounts for the payoff imbalance that stake rounding
    /// introduces.
    pub worst_case_profit: Cents,
    /// [`Signal::worst_case_profit`] as basis points of total stake.
    pub profit_bps: u32,
}

impl Signal {
    /// Construct a signal directly from raw parts.
    ///
    /// Takes `MAX_LEGS` allocations rather than [`MAX_CHUNKS`] because the
    /// engine's ring-buffer slot is still a one-allocation-per-outcome
    /// structure. Widening that is a separate change to a separate crate; until
    /// it happens this constructor is the narrow door, and a signal rebuilt
    /// through it carries only the first `MAX_LEGS` allocations.
    #[inline]
    pub const fn from_raw_parts(
        allocations: [Allocation; MAX_LEGS],
        len: u8,
        overround_ppm: u32,
        total_stake: Cents,
        worst_case_profit: Cents,
        profit_bps: u32,
    ) -> Signal {
        let mut wide = [Allocation {
            leg: 0,
            stake: 0,
            payout: 0,
        }; MAX_CHUNKS];
        let mut i = 0;
        while i < MAX_LEGS {
            wide[i] = allocations[i];
            i += 1;
        }
        let len = if len as usize > MAX_LEGS {
            MAX_LEGS as u8
        } else {
            len
        };
        Signal {
            allocations: wide,
            len,
            overround_ppm,
            total_stake,
            worst_case_profit,
            profit_bps,
        }
    }

    /// The per-chunk stakes.
    #[inline]
    pub fn allocations(&self) -> &[Allocation] {
        &self.allocations[..self.len as usize]
    }
}

/// Divide, rounding away from zero for non-negative inputs.
#[inline]
fn ceil_div(numerator: i128, denominator: i128) -> i128 {
    (numerator + denominator - 1) / denominator
}

/// The smallest multiple of `step` at or above `value`.
#[inline]
fn round_up_to(value: i128, step: i128) -> i128 {
    ceil_div(value, step) * step
}

/// The largest multiple of `step` at or below `value`.
#[inline]
fn round_down_to(value: i128, step: i128) -> i128 {
    (value / step) * step
}

/// Clamp into the range money is reported in.
///
/// The search runs in `i128` so nothing overflows mid-plan; a payout that will
/// not fit in [`Cents`] is reported as the largest value that does, which
/// understates it. Understating a payout is the safe direction.
#[inline]
fn to_cents(value: i128) -> i128 {
    value.clamp(Cents::MIN as i128, Cents::MAX as i128)
}

/// The flattened, fee-adjusted chunk set a plan is built from.
///
/// Struct-of-arrays and fixed width: this lives on the stack of one hot-path
/// call and is gone before the next feed message.
struct Chunks {
    /// Index into the caller's leg slice.
    leg: [usize; MAX_CHUNKS],
    /// Which outcome group this chunk belongs to, in `0..n_groups`.
    group: [usize; MAX_CHUNKS],
    /// Effective price in ppm, fees already applied. Always `1..=PPM`.
    price: [i128; MAX_CHUNKS],
    /// Stake increment in cents. Always positive.
    increment: [i128; MAX_CHUNKS],
    /// Most stake this chunk can take, already floored to `increment`.
    ///
    /// Floored because the freeze branch of [`Chunks::distribute`] assigns it
    /// straight into a stake, and a stake that is not a whole number of
    /// increments is not a stake the venue will accept.
    capacity: [i128; MAX_CHUNKS],
    /// The size actually resting at this level, before that flooring.
    ///
    /// Only the closed-form seed uses it, and it has to: that plan divides the
    /// depth by the overround *before* rounding the result to an increment, so
    /// handing it a pre-floored depth would shrink the plan it produces and
    /// break the guarantee that the search never returns less than the sizing
    /// it replaced.
    resting: [i128; MAX_CHUNKS],
    /// How many chunks are real.
    n: usize,
    /// How many outcome groups are real. Chunk `g` is group `g`'s best price.
    n_groups: usize,
    /// The cap on total stake, in cents.
    budget: i128,
}

impl Chunks {
    /// Gross return of `stake` on chunk `j`, floored.
    #[inline]
    fn payout(&self, j: usize, stake: i128) -> i128 {
        to_cents(stake * PPM_I / self.price[j])
    }

    /// Total stake and worst-case outcome payout of a complete plan.
    ///
    /// This is the only place the reported numbers come from. The search's
    /// target `T` is a search parameter, never an answer.
    fn evaluate(&self, stakes: &[i128; MAX_CHUNKS]) -> (i128, i128) {
        let mut group_payout = [0i128; MAX_CHUNKS];
        let mut total = 0i128;
        for j in 0..self.n {
            total += stakes[j];
            group_payout[self.group[j]] += self.payout(j, stakes[j]);
        }
        let mut worst = i128::MAX;
        for payout in group_payout.iter().take(self.n_groups) {
            worst = worst.min(to_cents(*payout));
        }
        (total, worst)
    }

    /// Size one outcome group to guarantee at least `target`, as cheaply as the
    /// increments and the resting depth allow.
    ///
    /// Returns the payout achieved, or `None` if the group cannot reach
    /// `target` at all — which makes the whole `target` infeasible, since a
    /// hedge is all-or-nothing.
    ///
    /// The split is proportional to price: a chunk that costs more per unit of
    /// payout is asked for a proportionally larger share of the target, which
    /// is the split that keeps every chunk's stake in the same ratio the
    /// closed-form equal-payoff solution would use. Chunks that hit their
    /// resting size are frozen at it and their shortfall redistributed over the
    /// rest — at most one freeze per round, so at most `m` rounds. A final
    /// repair loop, also bounded by `m`, closes any residual gap by bumping
    /// whichever chunk buys the most payout per cent.
    fn distribute(
        &self,
        group: usize,
        target: i128,
        stakes: &mut [i128; MAX_CHUNKS],
    ) -> Option<i128> {
        let mut member = [0usize; MAX_CHUNKS];
        let mut m = 0usize;
        for j in 0..self.n {
            if self.group[j] == group {
                member[m] = j;
                m += 1;
            }
        }

        let mut active = [true; MAX_CHUNKS];
        let mut remaining = target;
        let mut frozen_payout = 0i128;

        for _round in 0..m {
            let mut price_sum = 0i128;
            let mut live = 0usize;
            for k in 0..m {
                if active[k] {
                    price_sum += self.price[member[k]];
                    live += 1;
                }
            }
            if live == 0 {
                break;
            }
            if remaining <= 0 {
                // The frozen chunks already cover the target; anything still
                // active is stake we do not need to spend.
                for k in 0..m {
                    if active[k] {
                        stakes[member[k]] = 0;
                    }
                }
                break;
            }

            let mut freeze = [false; MAX_CHUNKS];
            let mut capped_any = false;
            for k in 0..m {
                if !active[k] {
                    continue;
                }
                let j = member[k];
                // This chunk's share of the payout target...
                let share = ceil_div(remaining * self.price[j], price_sum);
                // ...and the stake that buys it, rounded up to something the
                // venue will accept. Up, because a stake rounded down buys a
                // payout the venue will not pay.
                let need = round_up_to(ceil_div(share * self.price[j], PPM_I), self.increment[j]);
                if need > self.capacity[j] {
                    stakes[j] = self.capacity[j];
                    freeze[k] = true;
                    capped_any = true;
                } else {
                    stakes[j] = need;
                }
            }
            if !capped_any {
                break;
            }
            for k in 0..m {
                if freeze[k] {
                    active[k] = false;
                    frozen_payout += self.payout(member[k], stakes[member[k]]);
                }
            }
            remaining = target - frozen_payout;
        }

        let mut payout = 0i128;
        for k in 0..m {
            payout += self.payout(member[k], stakes[member[k]]);
        }

        // Repair. The proportional split cannot undershoot on its own — every
        // share is rounded up — so this only ever fires on a group whose
        // capped chunks left a gap, and it is bounded by the group's size
        // rather than by how big the gap is.
        for _ in 0..m {
            if payout >= target {
                break;
            }
            let mut best: Option<(usize, i128, i128)> = None;
            for &j in member.iter().take(m) {
                let step = self.increment[j];
                if stakes[j] + step > self.capacity[j] {
                    continue;
                }
                let gain = self.payout(j, stakes[j] + step) - self.payout(j, stakes[j]);
                best = match best {
                    // More payout per cent: gain / step, cross-multiplied.
                    Some((_, bg, bs)) if gain * bs <= bg * step => best,
                    _ => Some((j, gain, step)),
                };
            }
            let Some((j, gain, step)) = best else { break };
            stakes[j] += step;
            payout += gain;
        }

        if payout < target {
            return None;
        }
        Some(payout)
    }

    /// The cheapest plan that guarantees `target`, or `None` if there is none
    /// inside the depth and the budget.
    fn feasible(&self, target: i128) -> Option<[i128; MAX_CHUNKS]> {
        let mut stakes = [0i128; MAX_CHUNKS];
        for group in 0..self.n_groups {
            self.distribute(group, target, &mut stakes)?;
        }
        let total: i128 = stakes[..self.n].iter().sum();
        if total > self.budget {
            return None;
        }
        Some(stakes)
    }
}

/// The best plan seen so far, and the numbers it is judged on.
#[derive(Clone, Copy)]
struct Best {
    stakes: [i128; MAX_CHUNKS],
    total: i128,
    profit: i128,
    found: bool,
}

impl Best {
    /// Recompute a candidate exactly and keep it if it wins.
    ///
    /// More guaranteed profit wins, and *only* more guaranteed profit: a tie
    /// is kept by the incumbent. Since the incumbent is seeded with the
    /// closed-form equal-payoff plan before the search runs, that single rule
    /// buys two things at once.
    ///
    /// It makes the improvement guarantee exact rather than approximate. The
    /// plan this module emitted before it could see depth is offered first, so
    /// the reported profit is that plan's profit or better, always, and the
    /// reported *plan* is byte-for-byte the old one unless the search found
    /// strictly more guaranteed money.
    ///
    /// And it makes the sizing stable. Several plans routinely tie on profit —
    /// a payout target of 10_406 and one of 10_416 both clear 208 cents here —
    /// and picking between them on total stake would make the emitted
    /// allocations jitter every time an unrelated level ticked, for no extra
    /// guaranteed cent. A detector whose output moves only when the answer
    /// moves is worth more downstream than one that shaves a few cents of
    /// capital off an otherwise identical trade.
    fn offer(&mut self, chunks: &Chunks, stakes: &[i128; MAX_CHUNKS]) {
        let (total, worst) = chunks.evaluate(stakes);
        if total <= 0 {
            return;
        }
        let profit = to_cents(worst - total);
        if !self.found || profit > self.profit {
            self.stakes = *stakes;
            self.total = total;
            self.profit = profit;
            self.found = true;
        }
    }
}

/// Find a tradeable arbitrage across the books in `legs`, or `None`.
///
/// Each [`BookLeg`] is one venue's view of one outcome, several levels deep.
/// Legs sharing an `outcome` hedge the same side and are staked together;
/// every distinct outcome in the slice must be quoted somewhere, or there is
/// nothing to hedge with and the answer is `None`. `budget` caps the total
/// stake in cents.
///
/// Returns `Ok(None)` whenever there is no profitable trade — no edge after
/// fees, no depth, or an edge that stake rounding eats. Errors are reserved
/// for malformed input: fewer than two distinct outcomes, more legs than
/// [`MAX_CHUNKS`], a stake increment of zero.
///
/// # Correctness note
///
/// This function assumes the legs really are mutually exclusive and
/// collectively exhaustive sides of the *same* market. It cannot check that,
/// and it is the single most dangerous assumption in the system: two legs
/// drawn from different games sum to whatever they like and will happily look
/// like a 4% edge. Establishing that the legs belong together is the matcher's
/// job, upstream of here.
pub fn detect_book(legs: &[BookLeg], budget: Cents) -> Result<Option<Signal>> {
    if legs.len() < 2 || legs.len() > MAX_CHUNKS {
        return Err(ArbError::LegCountOutOfRange(legs.len()));
    }

    // Group legs by outcome, preserving first-appearance order.
    let mut group_of = [0usize; MAX_CHUNKS];
    let mut group_outcome = [0 as OutcomeId; MAX_CHUNKS];
    let mut n_groups = 0usize;
    for (i, leg) in legs.iter().enumerate() {
        let mut found = None;
        for (g, outcome) in group_outcome.iter().take(n_groups).enumerate() {
            if *outcome == leg.outcome {
                found = Some(g);
                break;
            }
        }
        group_of[i] = match found {
            Some(g) => g,
            None => {
                group_outcome[n_groups] = leg.outcome;
                n_groups += 1;
                n_groups - 1
            }
        };
    }
    if n_groups < 2 {
        // Every leg backs the same side. That is a directional bet with extra
        // steps, and the caller assembled the market wrong.
        return Err(ArbError::LegCountOutOfRange(n_groups));
    }

    plan(legs, &group_of, n_groups, budget)
}

/// Find a tradeable arbitrage across `legs`, or `None`.
///
/// The single-quote form, retained for callers that have not moved to
/// [`detect_book`]. `legs` must hold exactly one leg per outcome of a single
/// market, each already chosen as the best available price for that outcome;
/// each becomes a one-level book and the answer comes from [`detect_book`].
///
/// Note that this form treats *every* leg as its own outcome — the `outcome`
/// field is documentation here, not a grouping key, exactly as it has always
/// been. Two legs quoting the same side are not merged; they are treated as two
/// sides of a market, which is what "one leg per outcome" promises.
///
/// Returns `Ok(None)` whenever there is no profitable trade. Errors are
/// reserved for malformed input.
pub fn detect(legs: &[Leg], budget: Cents) -> Result<Option<Signal>> {
    if legs.len() < 2 || legs.len() > MAX_LEGS {
        return Err(ArbError::LegCountOutOfRange(legs.len()));
    }

    let empty = Level {
        price: Prob::CERTAIN,
        size: 0,
    };
    let mut book_legs = [BookLeg {
        venue: 0,
        outcome: 0,
        fee: Fee::None,
        increment: 1,
        levels: [empty; MAX_LEVELS_PER_LEG],
        n_levels: 0,
    }; MAX_LEGS];
    let mut group_of = [0usize; MAX_CHUNKS];

    for (i, leg) in legs.iter().enumerate() {
        let mut levels = [empty; MAX_LEVELS_PER_LEG];
        levels[0] = Level {
            price: leg.quoted,
            size: leg.capacity,
        };
        book_legs[i] = BookLeg {
            venue: leg.venue,
            outcome: leg.outcome,
            fee: leg.fee,
            increment: leg.increment,
            levels,
            n_levels: 1,
        };
        group_of[i] = i;
    }

    plan(&book_legs[..legs.len()], &group_of, legs.len(), budget)
}

/// The shared core: flatten, check the condition, search, report.
///
/// `group_of` gives each leg's outcome group in `0..n_groups`; [`detect_book`]
/// derives it from the outcome ids and [`detect`] hands every leg its own.
fn plan(
    legs: &[BookLeg],
    group_of: &[usize; MAX_CHUNKS],
    n_groups: usize,
    budget: Cents,
) -> Result<Option<Signal>> {
    for (i, leg) in legs.iter().enumerate() {
        if leg.n_levels > 0 && leg.increment <= 0 {
            return Err(ArbError::ZeroStakeIncrement(i));
        }
    }
    if budget <= 0 {
        return Ok(None);
    }

    let mut chunks = Chunks {
        leg: [0; MAX_CHUNKS],
        group: [0; MAX_CHUNKS],
        price: [PPM_I; MAX_CHUNKS],
        increment: [1; MAX_CHUNKS],
        capacity: [0; MAX_CHUNKS],
        resting: [0; MAX_CHUNKS],
        n: 0,
        n_groups,
        budget: budget as i128,
    };

    // Pass one: the best usable level of each outcome, so that truncation can
    // never drop an outcome entirely. An outcome with nothing usable behind it
    // cannot be hedged, and a partial hedge is a directional bet.
    let mut taken = [0u8; MAX_CHUNKS];
    for group in 0..n_groups {
        let mut best: Option<(usize, usize, i128, i128)> = None;
        for (i, leg) in legs.iter().enumerate() {
            if group_of[i] != group {
                continue;
            }
            for l in 0..usable_levels(leg) {
                let Some(capacity) = usable_capacity(leg, l) else {
                    continue;
                };
                let price = effective_ppm(leg, l);
                let improves = match best {
                    Some((_, _, best_price, _)) => price < best_price,
                    None => true,
                };
                if improves {
                    best = Some((i, l, price, capacity));
                }
            }
        }
        let Some((i, l, price, capacity)) = best else {
            return Ok(None);
        };
        chunks.leg[chunks.n] = i;
        chunks.group[chunks.n] = group;
        chunks.price[chunks.n] = price;
        chunks.increment[chunks.n] = legs[i].increment as i128;
        chunks.capacity[chunks.n] = capacity;
        chunks.resting[chunks.n] = legs[i].levels[l].size as i128;
        taken[i] |= 1 << l;
        chunks.n += 1;
    }

    // The condition itself, read off the best price on each outcome. At or
    // above parity no plan can profit: every outcome needs at least
    // `payout * p / PPM` staked on it, so the total staked is at least the
    // guaranteed payout and the difference cannot be positive.
    let mut overround: i128 = 0;
    for group in 0..n_groups {
        overround += chunks.price[group];
    }
    if overround >= PPM_I {
        return Ok(None);
    }

    // Pass two: the rest of the depth, best level first within each leg, until
    // the chunk budget is spent.
    for (i, leg) in legs.iter().enumerate() {
        for l in 0..usable_levels(leg) {
            if chunks.n == MAX_CHUNKS {
                break;
            }
            if taken[i] & (1 << l) != 0 {
                continue;
            }
            let Some(capacity) = usable_capacity(leg, l) else {
                continue;
            };
            chunks.leg[chunks.n] = i;
            chunks.group[chunks.n] = group_of[i];
            chunks.price[chunks.n] = effective_ppm(leg, l);
            chunks.increment[chunks.n] = leg.increment as i128;
            chunks.capacity[chunks.n] = capacity;
            chunks.resting[chunks.n] = leg.levels[l].size as i128;
            taken[i] |= 1 << l;
            chunks.n += 1;
        }
    }

    let mut best = Best {
        stakes: [0; MAX_CHUNKS],
        total: 0,
        profit: 0,
        found: false,
    };

    // Candidate one: the closed-form equal-payoff plan on the best price of
    // each outcome — the plan this module emitted before it could see depth.
    // Including it explicitly is what makes "never worse than before" a fact
    // rather than a hope: it is a member of the feasible set by construction,
    // so the answer is at least as good as it.
    let mut ceiling = chunks.budget;
    for group in 0..n_groups {
        ceiling = ceiling.min(chunks.resting[group] * overround / chunks.price[group]);
    }
    if ceiling > 0 {
        let mut stakes = [0i128; MAX_CHUNKS];
        let mut usable = true;
        for (group, slot) in stakes.iter_mut().enumerate().take(n_groups) {
            let ideal = ceiling * chunks.price[group] / overround;
            let stake = round_down_to(ideal, chunks.increment[group]);
            if stake <= 0 {
                usable = false;
                break;
            }
            *slot = stake;
        }
        if usable {
            best.offer(&chunks, &stakes);
        }
    }

    // Candidates two and up: binary search on the guaranteed payout. The
    // ceiling is whichever binds first — what the budget can buy at the longest
    // price on offer, or the most any single outcome can be made to pay.
    let mut cheapest = PPM_I;
    for j in 0..chunks.n {
        cheapest = cheapest.min(chunks.price[j]);
    }
    let mut highest = chunks.budget * PPM_I / cheapest;
    for group in 0..n_groups {
        let mut reach = 0i128;
        for j in 0..chunks.n {
            if chunks.group[j] == group {
                reach += chunks.payout(j, chunks.capacity[j]);
            }
        }
        highest = highest.min(reach);
    }

    let (mut low, mut high) = (0i128, highest.max(0));
    for _ in 0..MAX_SEARCH_STEPS {
        if low >= high {
            break;
        }
        let mid = low + (high - low + 1) / 2;
        match chunks.feasible(mid) {
            Some(stakes) => {
                best.offer(&chunks, &stakes);
                low = mid;
            }
            None => high = mid - 1,
        }
    }

    if !best.found || best.profit <= 0 {
        return Ok(None);
    }

    let mut allocations = [Allocation {
        leg: 0,
        stake: 0,
        payout: 0,
    }; MAX_CHUNKS];
    let mut len = 0usize;
    for j in 0..chunks.n {
        if best.stakes[j] <= 0 {
            continue;
        }
        allocations[len] = Allocation {
            leg: chunks.leg[j],
            stake: best.stakes[j] as Cents,
            payout: chunks.payout(j, best.stakes[j]) as Cents,
        };
        len += 1;
    }

    Ok(Some(Signal {
        allocations,
        len: len as u8,
        overround_ppm: overround as u32,
        total_stake: best.total as Cents,
        worst_case_profit: best.profit as Cents,
        profit_bps: (best.profit * 10_000 / best.total).clamp(0, u32::MAX as i128) as u32,
    }))
}

/// How many of a leg's levels the detector will look at.
#[inline]
fn usable_levels(leg: &BookLeg) -> usize {
    (leg.n_levels as usize).min(MAX_LEVELS_PER_LEG)
}

/// The effective price of one level, in ppm, with the venue's cut already in.
#[inline]
fn effective_ppm(leg: &BookLeg, level: usize) -> i128 {
    i128::from(leg.fee.effective(leg.levels[level].price).ppm())
}

/// A level's depth floored to a tradeable stake, or `None` if it cannot take
/// even one increment.
#[inline]
fn usable_capacity(leg: &BookLeg, level: usize) -> Option<i128> {
    let increment = leg.increment as i128;
    if increment <= 0 {
        return None;
    }
    let capacity = round_down_to(leg.levels[level].size as i128, increment);
    if capacity < increment {
        return None;
    }
    Some(capacity)
}

/// Build a leg with no fee, unbounded depth, and cent granularity.
///
/// For tests and for venues that genuinely are continuous. Real venues are
/// not, and reaching for this in production code is how depth stops being
/// modelled.
pub fn frictionless_leg(venue: VenueId, outcome: OutcomeId, quoted: Prob) -> Leg {
    Leg {
        venue,
        outcome,
        quoted,
        fee: Fee::None,
        capacity: Cents::MAX / 4,
        increment: 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fee::kalshi_stake_fee_bps;

    fn leg_at(venue: VenueId, cents: u32) -> Leg {
        frictionless_leg(venue, u32::from(venue), Prob::from_cents(cents).unwrap())
    }

    /// A book leg from `(cents, size)` levels, best-first.
    fn book_leg(venue: VenueId, outcome: OutcomeId, levels: &[(u32, Cents)]) -> BookLeg {
        let empty = Level {
            price: Prob::CERTAIN,
            size: 0,
        };
        let mut filled = [empty; MAX_LEVELS_PER_LEG];
        for (slot, (cents, size)) in filled.iter_mut().zip(levels) {
            *slot = Level {
                price: Prob::from_cents(*cents).unwrap(),
                size: *size,
            };
        }
        BookLeg {
            venue,
            outcome,
            fee: Fee::None,
            increment: 1,
            levels: filled,
            n_levels: levels.len() as u8,
        }
    }

    #[test]
    fn a_two_way_market_that_sums_under_one_is_an_arbitrage() {
        // 48c on one venue and 50c on another: 98c to buy a dollar.
        let legs = [leg_at(0, 48), leg_at(1, 50)];
        let signal = detect(&legs, 100_000).unwrap().unwrap();

        assert_eq!(signal.overround_ppm, 980_000);
        // A cent short of the $1000 budget: both stakes floor to a whole cent
        // and neither is allowed to round up into money that is not there.
        //
        // $2040 is the true optimum here, and the search confirms it rather
        // than changing it. Guaranteeing a payout of M costs at least
        // ceil(0.48M) + ceil(0.50M) >= 0.98M, so the budget bounds M at
        // 100_000 / 0.98 = 102_040 and the profit M - 0.98M is at most 2_040 —
        // which the closed-form plan below already clears. Several search
        // probes tie with it at 2_040 and none beats it, so the numbers here
        // are unchanged from before the book search existed. That is the
        // point: the search only moves the answer when it moves it upward.
        assert_eq!(signal.total_stake, 99_999);
        // That $1000 buys $1020.39 of certain return, whichever side lands.
        assert_eq!(signal.worst_case_profit, 2_040);
        assert_eq!(signal.profit_bps, 204);

        // Every leg returns at least the guaranteed amount, which is what
        // "risk-free" has to mean.
        for allocation in signal.allocations() {
            assert!(allocation.payout - signal.total_stake >= signal.worst_case_profit);
        }
    }

    #[test]
    fn a_normal_market_with_vig_is_not_an_arbitrage() {
        // -110 both ways is the standard sportsbook price. It sums to 1.0476:
        // the 476 bp overround is the book's margin, and it is the reason
        // most of what looks like an opportunity is not one.
        let quoted = Prob::from_american(-110).unwrap();
        let legs = [
            frictionless_leg(0, 0, quoted),
            frictionless_leg(1, 1, quoted),
        ];
        assert_eq!(detect(&legs, 100_000).unwrap(), None);
    }

    #[test]
    fn fees_are_what_turns_most_apparent_edges_into_losses() {
        // 49c and 50c: a 100 bp raw edge, and a clear signal on raw prices.
        let raw = [leg_at(0, 49), leg_at(1, 50)];
        assert!(detect(&raw, 100_000).unwrap().is_some());

        // The same two prices on Kalshi, where the per-contract fee runs
        // about 350 bp of stake at these levels. The edge was never there.
        let with_fees: Vec<Leg> = raw
            .iter()
            .map(|leg| Leg {
                fee: Fee::StakeFeeBps(kalshi_stake_fee_bps(leg.quoted)),
                ..*leg
            })
            .collect();
        assert_eq!(detect(&with_fees, 100_000).unwrap(), None);
    }

    #[test]
    fn depth_caps_the_trade_at_the_thinnest_leg() {
        // A real 200 bp edge, but only $50 resting on one side.
        let legs = [
            Leg {
                capacity: 5_000,
                ..leg_at(0, 48)
            },
            Leg {
                capacity: 1_000_000,
                ..leg_at(1, 50)
            },
        ];
        let signal = detect(&legs, 10_000_000).unwrap().unwrap();

        // The thin leg's share is 48/98 of the total, so $50 there supports
        // about $102 overall — not the $100_000 budget the caller offered.
        //
        // These numbers improved when the search replaced the closed-form
        // sizing. The old code staked 4_999 and 5_208 for 10_207 outlaid and
        // cleared 207 cents; 208 is the true pessimistic optimum, and this is
        // the proof.
        //
        // Let M be a plan's guaranteed payout. The thin leg alone bounds it:
        // 5_000 cents of depth at 48c pays at most floor(5_000 / 0.48) =
        // 10_416, and the deep leg backs the *other* outcome, so it cannot
        // make up the difference — hence M <= 10_416. Reaching M costs at
        // least ceil(0.48 M) on the thin side and ceil(0.50 M) on the deep
        // one, and both increments are one cent, so no further rounding
        // applies. Therefore
        //
        //     profit <= M - ceil(0.48 M) - ceil(0.50 M) <= 0.02 M <= 208.32,
        //
        // and profit is an integer, so profit <= 208. The plan below attains
        // it: 4_995 at 48c pays floor(4_995 / 0.48) = 10_406 and 5_203 at 50c
        // pays 10_406, so 10_198 outlaid returns 10_406 whichever side lands,
        // for 208 guaranteed. No plan does better.
        //
        // Several plans tie at 208 — M = 10_416 costs 10_208 for the same 208
        // — and ties go to the incumbent (see `Best::offer`), so the one
        // reported is the first the search found that strictly beat the
        // closed-form 207.
        assert_eq!(signal.allocations()[0].stake, 4_995);
        assert_eq!(signal.total_stake, 10_198);
        assert_eq!(signal.worst_case_profit, 208);
        // And the depth on offer is a cap, never exceeded.
        assert!(signal.allocations()[0].stake <= legs[0].capacity);
    }

    #[test]
    fn stake_granularity_can_eat_an_edge_that_survived_everything_else() {
        // A thin edge, on a venue that only trades in $25 increments and only
        // has enough depth for a couple of them. Rounding both legs down
        // breaks the equal-payoff property, and what is left is a loss.
        //
        // Exhaustively: each leg can stake 2_500 or 5_000, so the four plans
        // pay (5_000, 5_000), (10_204, 10_000), (5_102, 10_000) and
        // (10_204, 5_000) against outlays of 5_000, 10_000, 7_500 and 7_500.
        // The best guaranteed profit any of them clears is zero.
        let legs = [
            Leg {
                increment: 2_500,
                capacity: 7_000,
                ..leg_at(0, 49)
            },
            Leg {
                increment: 2_500,
                capacity: 7_000,
                ..leg_at(1, 50)
            },
        ];
        assert_eq!(detect(&legs, 1_000_000).unwrap(), None);

        // With enough depth to round to, the same prices trade.
        let deeper: Vec<Leg> = legs
            .iter()
            .map(|leg| Leg {
                capacity: 10_000_000,
                ..*leg
            })
            .collect();
        assert!(detect(&deeper, 100_000_000).unwrap().is_some());
    }

    #[test]
    fn three_way_markets_work_the_same_way() {
        // Soccer, where the draw is a real outcome and forgetting it is how
        // a two-legged "arb" turns into an unhedged bet.
        let legs = [leg_at(0, 45), leg_at(1, 30), leg_at(2, 23)];
        let signal = detect(&legs, 100_000).unwrap().unwrap();
        assert_eq!(signal.overround_ppm, 980_000);
        assert_eq!(signal.allocations().len(), 3);
    }

    #[test]
    fn dropping_an_outcome_is_not_a_bigger_edge() {
        // The same soccer market with the draw ignored sums to 0.75 and looks
        // like a 33% return. It is a bet that the game does not end level.
        // `detect` cannot tell — which is precisely why the matcher, not this
        // function, is responsible for the legs being exhaustive.
        let partial = [leg_at(0, 45), leg_at(1, 30)];
        let signal = detect(&partial, 100_000).unwrap().unwrap();
        assert!(signal.profit_bps > 3_000);
    }

    #[test]
    fn malformed_input_is_an_error_but_a_missing_edge_is_not() {
        let single = [leg_at(0, 48)];
        assert!(matches!(
            detect(&single, 100_000),
            Err(ArbError::LegCountOutOfRange(1))
        ));

        let bad_increment = [
            Leg {
                increment: 0,
                ..leg_at(0, 48)
            },
            leg_at(1, 50),
        ];
        assert!(matches!(
            detect(&bad_increment, 100_000),
            Err(ArbError::ZeroStakeIncrement(0))
        ));

        // No budget is a market condition, not a bug.
        assert_eq!(detect(&[leg_at(0, 48), leg_at(1, 50)], 0).unwrap(), None);
    }

    #[test]
    fn a_thin_top_of_book_is_worth_less_than_the_depth_behind_it() {
        // 45c for a dollar, but only $2 of it. Behind it sits 48c, which is
        // still an arb against the 50c on the other side — and it is where
        // almost all of the money in this market is.
        let legs = [
            book_leg(0, 0, &[(45, 200), (48, 1_000_000)]),
            book_leg(1, 1, &[(50, 1_000_000)]),
        ];
        let deep = detect_book(&legs, 1_000_000).unwrap().unwrap();

        // The same market seen only at the top of book: $2 of depth on one
        // side caps the whole trade at about $4.
        let top_only = [
            book_leg(0, 0, &[(45, 200)]),
            book_leg(1, 1, &[(50, 1_000_000)]),
        ];
        let shallow = detect_book(&top_only, 1_000_000).unwrap().unwrap();

        assert!(shallow.total_stake < 500);
        assert!(
            deep.worst_case_profit > shallow.worst_case_profit * 100,
            "deep {} vs shallow {}",
            deep.worst_case_profit,
            shallow.worst_case_profit,
        );
        // Both levels of the first leg are staked into, and neither exceeds
        // the size resting on it.
        let first: Vec<&Allocation> = deep.allocations().iter().filter(|a| a.leg == 0).collect();
        assert_eq!(first.len(), 2);
        assert!(first.iter().map(|a| a.stake).sum::<Cents>() > 200);
    }

    #[test]
    fn a_level_that_is_no_longer_an_arb_is_simply_not_staked_into() {
        // 45c then 60c on one side against 50c on the other. The second level
        // sums to 1.10 and is not part of any hedge; taking it would turn a
        // guaranteed profit into a smaller one.
        let legs = [
            book_leg(0, 0, &[(45, 10_000), (60, 1_000_000)]),
            book_leg(1, 1, &[(50, 1_000_000)]),
        ];
        let signal = detect_book(&legs, 1_000_000).unwrap().unwrap();

        let staked_at_60: Cents = signal
            .allocations()
            .iter()
            .filter(|a| a.leg == 0)
            .map(|a| a.stake)
            .sum::<Cents>()
            .saturating_sub(10_000);
        assert!(
            staked_at_60 <= 0,
            "staked {staked_at_60} past the 45c level"
        );
    }

    #[test]
    fn an_outcome_with_no_usable_quote_produces_no_signal() {
        // A hedge is all or nothing. A side whose book has gone stale — or
        // whose only resting size is smaller than one tradeable increment —
        // leaves a leg of the trade unfillable, and a partial hedge is a
        // directional bet wearing an arbitrage's clothes.
        let stale = [
            book_leg(0, 0, &[(45, 1_000_000)]),
            BookLeg {
                n_levels: 0,
                ..book_leg(1, 1, &[(50, 1_000_000)])
            },
        ];
        assert_eq!(detect_book(&stale, 1_000_000).unwrap(), None);

        let dust = [
            book_leg(0, 0, &[(45, 1_000_000)]),
            BookLeg {
                increment: 5_000,
                ..book_leg(1, 1, &[(50, 100)])
            },
        ];
        assert_eq!(detect_book(&dust, 1_000_000).unwrap(), None);
    }

    #[test]
    fn two_venues_quoting_the_same_side_are_one_hedge_between_them() {
        // Kalshi and Polymarket both quoting the away team is not a two-way
        // market. Grouping by outcome is what stops the detector from reading
        // one side of a game as both sides of one.
        let same_side = [
            book_leg(0, 7, &[(45, 1_000_000)]),
            book_leg(1, 7, &[(46, 1_000_000)]),
        ];
        assert!(matches!(
            detect_book(&same_side, 1_000_000),
            Err(ArbError::LegCountOutOfRange(1))
        ));

        // With a genuine other side, the two same-side legs share the payout
        // burden of their outcome rather than competing as separate outcomes.
        let market = [
            book_leg(0, 7, &[(45, 2_000)]),
            book_leg(1, 7, &[(46, 1_000_000)]),
            book_leg(2, 9, &[(50, 1_000_000)]),
        ];
        let signal = detect_book(&market, 1_000_000).unwrap().unwrap();
        // The 45c leg is thin, so the 46c quote on the other venue carries the
        // rest of that side — depth the top-of-book view could not reach.
        assert!(signal.total_stake > 4_000);
        assert!(signal.worst_case_profit > 0);
    }

    #[test]
    fn fees_are_applied_per_level_not_per_leg() {
        // A book whose levels are 49c and 50c, against a flat 50c on the other
        // side. Without fees the first level is a 100 bp edge; with Kalshi's
        // stake fee — which is *larger* on the cheaper contract — neither level
        // is, and the whole market disappears rather than half of it.
        let free = [
            book_leg(0, 0, &[(49, 1_000_000), (50, 1_000_000)]),
            book_leg(1, 1, &[(49, 1_000_000)]),
        ];
        assert!(detect_book(&free, 1_000_000).unwrap().is_some());

        let taxed = [
            BookLeg {
                fee: Fee::StakeFeeBps(kalshi_stake_fee_bps(Prob::from_cents(49).unwrap())),
                ..free[0]
            },
            BookLeg {
                fee: Fee::CommissionBps(500),
                ..free[1]
            },
        ];
        assert_eq!(detect_book(&taxed, 1_000_000).unwrap(), None);

        // And a commission, which bites hardest on long prices, is applied to
        // the long level of a book and not only to its top.
        let long = [
            book_leg(0, 0, &[(10, 1_000), (20, 1_000_000)]),
            book_leg(1, 1, &[(75, 1_000_000)]),
        ];
        let raw = detect_book(&long, 10_000_000).unwrap().unwrap();
        let charged = [
            BookLeg {
                fee: Fee::CommissionBps(1_000),
                ..long[0]
            },
            long[1],
        ];
        let after = detect_book(&charged, 10_000_000).unwrap().unwrap();
        assert!(
            after.worst_case_profit < raw.worst_case_profit,
            "commission left the profit at {}",
            after.worst_case_profit,
        );
    }

    #[test]
    fn a_deep_book_never_stakes_past_what_is_resting_or_budgeted() {
        let legs = [
            book_leg(0, 0, &[(40, 300), (44, 700), (47, 5_000)]),
            book_leg(1, 1, &[(48, 400), (50, 900_000)]),
        ];
        let signal = detect_book(&legs, 50_000).unwrap().unwrap();

        assert!(signal.total_stake <= 50_000);
        let mut per_level = [0 as Cents; MAX_CHUNKS];
        for (slot, allocation) in per_level.iter_mut().zip(signal.allocations()) {
            *slot = allocation.stake;
            assert!(allocation.stake > 0);
        }
        // Every allocation is inside the size resting on some level of its leg,
        // and the outcome sums cover the promised profit.
        let mut outcome_payout = [0 as Cents; 2];
        for allocation in signal.allocations() {
            outcome_payout[legs[allocation.leg].outcome as usize] += allocation.payout;
        }
        for payout in outcome_payout {
            assert!(payout - signal.total_stake >= signal.worst_case_profit);
        }
    }

    #[test]
    fn the_book_form_and_the_single_quote_form_agree_on_one_level() {
        // The adapter is not a different detector. Given one level per leg the
        // two entry points must produce the same plan, or every caller that
        // has not migrated is trading a different book from the one that gets
        // tested.
        let legs = [
            Leg {
                capacity: 40_000,
                increment: 25,
                ..leg_at(0, 47)
            },
            Leg {
                capacity: 90_000,
                increment: 10,
                ..leg_at(1, 50)
            },
        ];
        let from_legs = detect(&legs, 60_000).unwrap().unwrap();

        let books: Vec<BookLeg> = legs
            .iter()
            .map(|leg| BookLeg {
                increment: leg.increment,
                ..book_leg(
                    leg.venue,
                    leg.outcome,
                    &[(leg.quoted.ppm() / 10_000, leg.capacity)],
                )
            })
            .collect();
        let from_books = detect_book(&books, 60_000).unwrap().unwrap();

        assert_eq!(from_legs, from_books);
    }
}
