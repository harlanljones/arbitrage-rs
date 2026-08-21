//! Per-venue capital ledger for the paper trading simulator.
//!
//! A real arbitrage book is never one undifferentiated pile of money: capital
//! sits at each venue separately, and a leg can only be staked with whatever
//! is actually sitting at that venue's account. `Bankroll` tracks that split
//! (`available` vs. `locked`) per venue so the simulator can refuse a trade it
//! cannot actually afford instead of reporting a paper profit that real
//! capital constraints would have blocked.
//!
//! # Why `available` / `locked`, not one balance
//!
//! Money moves through three states around an execution:
//!
//! 1. **Available** — free to stake.
//! 2. **Locked** — reserved against an order that has been sent but not yet
//!    settled (either because it is resting as an open order, or because it
//!    filled and is now collateral against a future outcome).
//! 3. **Gone / credited** — at settlement, a losing leg's locked stake is
//!    consumed; a winning leg's locked stake is released and the payout is
//!    credited back to `available`.
//!
//! Collapsing this to a single balance would let the simulator "spend" money
//! that is actually tied up in a resting order, understating how quickly a
//! bankroll gets exhausted under partial fills.
//!
//! # No allocation
//!
//! Per the workspace rule, this type preallocates: capital is tracked in
//! fixed `[Cents; MAX_BANKROLL_VENUES]` arrays indexed directly by `VenueId`
//! cast to `usize`. There is no `Vec`, no `HashMap`, and no heap growth as
//! venues are added — the venue count is bounded at construction time.
//!
//! # Pessimistic handling of out-of-range venues
//!
//! `VenueId` is a `u16` and nothing stops a caller from asking about a venue
//! index at or beyond `MAX_BANKROLL_VENUES`. Rather than panicking on the hot
//! (well, warm — this is the sim crate) path, an out-of-range venue is
//! treated as a venue with zero capital: `available` reports `0`, `reserve`
//! reports `false`, and the settlement methods are no-ops. This is the same
//! "when in doubt, be pessimistic" posture used everywhere else in the
//! system — an unrecognized venue can never fund a trade.

use arbkit_core::{Cents, VenueId};

use crate::error::BankrollError;

/// Maximum number of distinct venues a single [`Bankroll`] can track capital
/// for. Chosen to comfortably exceed any realistic venue count while keeping
/// the ledger's footprint small and fixed-size.
pub const MAX_BANKROLL_VENUES: usize = 32;

/// One basis point denominator (100% = 10,000 bps), matching `accounting.rs`.
const BPS: i128 = 10_000;

/// Basis points of settlement friction applied to a winning payout before it
/// is credited to `available`.
///
/// This is a hook for a later "transfer cost" flag (withdrawal fees, wire
/// costs, cross-venue rebalancing) — it is wired into [`Bankroll::settle_win`]
/// today but defaults to zero, so it has no observable effect until a future
/// workstream turns it on.
const TRANSFER_FRICTION_BPS: i128 = 0;

/// Per-venue capital ledger: `available` (free to stake) and `locked`
/// (reserved against an in-flight or settling order) balances, in cents.
///
/// See the module docs for the state machine `available` / `locked` money
/// moves through, and for why out-of-range venues degrade to "zero capital"
/// rather than panicking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bankroll {
    available: [Cents; MAX_BANKROLL_VENUES],
    locked: [Cents; MAX_BANKROLL_VENUES],
}

impl Bankroll {
    /// Construct a bankroll from a per-venue initial balance table.
    ///
    /// `initial_per_venue[i]` is the opening `available` balance for venue
    /// `i` (i.e. the slice index *is* the venue id). Venues beyond the slice
    /// length start at zero.
    ///
    /// # Errors
    ///
    /// Returns [`BankrollError::TooManyVenues`] if more than
    /// [`MAX_BANKROLL_VENUES`] balances are supplied, and
    /// [`BankrollError::NegativeInitialBalance`] if any balance is negative.
    /// Malformed input is rejected here rather than silently clamped, per the
    /// workspace rule that construction errors are for malformed input only.
    pub fn new(initial_per_venue: &[Cents]) -> Result<Self, BankrollError> {
        if initial_per_venue.len() > MAX_BANKROLL_VENUES {
            return Err(BankrollError::TooManyVenues(initial_per_venue.len()));
        }

        let mut available = [0 as Cents; MAX_BANKROLL_VENUES];
        for (i, &balance) in initial_per_venue.iter().enumerate() {
            if balance < 0 {
                return Err(BankrollError::NegativeInitialBalance {
                    venue: i,
                    cents: balance,
                });
            }
            available[i] = balance;
        }

        Ok(Self {
            available,
            locked: [0; MAX_BANKROLL_VENUES],
        })
    }

    /// Map a `VenueId` to an array index, or `None` if it is out of range.
    #[inline]
    fn index(venue: VenueId) -> Option<usize> {
        let idx = venue as usize;
        (idx < MAX_BANKROLL_VENUES).then_some(idx)
    }

    /// Free capital at `venue`, in cents. Zero for an out-of-range venue.
    #[inline]
    pub fn available(&self, venue: VenueId) -> Cents {
        Self::index(venue).map(|i| self.available[i]).unwrap_or(0)
    }

    /// Capital locked (reserved, pending settlement) at `venue`, in cents.
    /// Zero for an out-of-range venue.
    #[inline]
    pub fn locked(&self, venue: VenueId) -> Cents {
        Self::index(venue).map(|i| self.locked[i]).unwrap_or(0)
    }

    /// Reserve `amount` of `venue`'s available capital ahead of sending an
    /// order.
    ///
    /// Returns `false` if the venue is out of range, or if `amount` exceeds
    /// what is currently available — in either case the caller must skip the
    /// trade and record a capital-short disposition; state is left
    /// unchanged. A non-positive `amount` trivially succeeds without moving
    /// any money.
    pub fn reserve(&mut self, venue: VenueId, amount: Cents) -> bool {
        let Some(i) = Self::index(venue) else {
            return false;
        };
        if amount <= 0 {
            return true;
        }
        if self.available[i] < amount {
            return false;
        }
        self.available[i] -= amount;
        self.locked[i] = self.locked[i].saturating_add(amount);
        true
    }

    /// Reconcile a fill report against a prior [`reserve`](Self::reserve).
    ///
    /// The `filled` portion of the reservation stays locked, as collateral
    /// against the pending outcome. The `unfilled` remainder is released
    /// back to `available`. Amounts are clamped pessimistically: a negative
    /// `unfilled` is treated as zero, and the refund never exceeds what is
    /// actually locked at the venue (so a caller passing a bogus `unfilled`
    /// cannot manufacture capital).
    pub fn commit_fill(&mut self, venue: VenueId, filled: Cents, unfilled: Cents) {
        let Some(i) = Self::index(venue) else {
            return;
        };
        let _ = filled; // Already resident in `locked` from `reserve`; nothing to do.
        let unfilled = unfilled.max(0);
        let refund = unfilled.min(self.locked[i]);
        self.locked[i] -= refund;
        self.available[i] = self.available[i].saturating_add(refund);
    }

    /// Settle a losing leg: `locked` cents of collateral are consumed
    /// (transferred out of the system to the counterparty) and do not return
    /// to `available`.
    ///
    /// `locked` is clamped to what is actually locked at the venue, so this
    /// can never drive a venue's locked balance negative.
    pub fn settle_loss(&mut self, venue: VenueId, locked: Cents) {
        let Some(i) = Self::index(venue) else {
            return;
        };
        let locked = locked.max(0).min(self.locked[i]);
        self.locked[i] -= locked;
    }

    /// Settle a winning leg: `locked` cents of collateral are released, and
    /// `payout` cents are credited to `available` (net of any configured
    /// transfer friction, applied pessimistically — the friction cost always
    /// rounds in the venue's favor, never the bankroll's).
    pub fn settle_win(&mut self, venue: VenueId, locked: Cents, payout: Cents) {
        let Some(i) = Self::index(venue) else {
            return;
        };
        let locked = locked.max(0).min(self.locked[i]);
        self.locked[i] -= locked;
        let credited = apply_transfer_friction(payout.max(0));
        self.available[i] = self.available[i].saturating_add(credited);
    }

    /// Sum of `available` across all venues, in cents.
    pub fn total_available(&self) -> Cents {
        self.available
            .iter()
            .copied()
            .fold(0 as Cents, |acc, c| acc.saturating_add(c))
    }

    /// Sum of `locked` across all venues, in cents.
    pub fn total_locked(&self) -> Cents {
        self.locked
            .iter()
            .copied()
            .fold(0 as Cents, |acc, c| acc.saturating_add(c))
    }
}

/// Apply the (currently zero) transfer-friction haircut to a payout before
/// crediting it to `available`. Pessimistic: friction always floors the
/// credited amount, never rounds it up.
#[inline]
fn apply_transfer_friction(payout: Cents) -> Cents {
    if TRANSFER_FRICTION_BPS == 0 {
        return payout;
    }
    let kept_bps = BPS - TRANSFER_FRICTION_BPS;
    ((payout as i128 * kept_bps) / BPS) as Cents
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn new_rejects_too_many_venues() {
        let balances = vec![0 as Cents; MAX_BANKROLL_VENUES + 1];
        let err = Bankroll::new(&balances).unwrap_err();
        assert_eq!(err, BankrollError::TooManyVenues(MAX_BANKROLL_VENUES + 1));
    }

    #[test]
    fn new_rejects_negative_balance() {
        let balances = [10_000, -1, 5_000];
        let err = Bankroll::new(&balances).unwrap_err();
        assert_eq!(
            err,
            BankrollError::NegativeInitialBalance {
                venue: 1,
                cents: -1
            }
        );
    }

    #[test]
    fn new_accepts_valid_balances_and_zero_fills_the_rest() {
        let bankroll = Bankroll::new(&[10_000, 5_000]).unwrap();
        assert_eq!(bankroll.available(0), 10_000);
        assert_eq!(bankroll.available(1), 5_000);
        assert_eq!(bankroll.available(2), 0);
        assert_eq!(bankroll.total_available(), 15_000);
        assert_eq!(bankroll.total_locked(), 0);
    }

    #[test]
    fn reserve_insufficient_balance_returns_false_and_is_a_no_op() {
        let mut bankroll = Bankroll::new(&[1_000]).unwrap();
        let before = bankroll;

        assert!(!bankroll.reserve(0, 1_001));
        assert_eq!(bankroll, before);
    }

    #[test]
    fn reserve_out_of_range_venue_returns_false() {
        let mut bankroll = Bankroll::new(&[1_000]).unwrap();
        assert!(!bankroll.reserve(MAX_BANKROLL_VENUES as VenueId, 1));
    }

    #[test]
    fn reserve_then_commit_full_fill_keeps_stake_locked() {
        let mut bankroll = Bankroll::new(&[10_000]).unwrap();
        assert!(bankroll.reserve(0, 4_000));
        assert_eq!(bankroll.available(0), 6_000);
        assert_eq!(bankroll.locked(0), 4_000);

        bankroll.commit_fill(0, 4_000, 0);
        assert_eq!(bankroll.available(0), 6_000);
        assert_eq!(bankroll.locked(0), 4_000);
    }

    #[test]
    fn reserve_then_commit_partial_fill_refunds_remainder() {
        let mut bankroll = Bankroll::new(&[10_000]).unwrap();
        assert!(bankroll.reserve(0, 4_000));

        bankroll.commit_fill(0, 2_500, 1_500);
        assert_eq!(bankroll.available(0), 7_500);
        assert_eq!(bankroll.locked(0), 2_500);
    }

    #[test]
    fn settle_loss_consumes_locked_capital_permanently() {
        let mut bankroll = Bankroll::new(&[10_000]).unwrap();
        bankroll.reserve(0, 4_000);
        bankroll.settle_loss(0, 4_000);

        assert_eq!(bankroll.locked(0), 0);
        assert_eq!(bankroll.available(0), 6_000);
        assert_eq!(bankroll.total_available() + bankroll.total_locked(), 6_000);
    }

    #[test]
    fn settle_win_releases_locked_and_credits_payout() {
        let mut bankroll = Bankroll::new(&[10_000]).unwrap();
        bankroll.reserve(0, 4_000);
        bankroll.settle_win(0, 4_000, 4_400);

        assert_eq!(bankroll.locked(0), 0);
        assert_eq!(bankroll.available(0), 6_000 + 4_400);
    }

    #[test]
    fn failed_reservation_leaves_bankroll_untouched_for_capital_short_bookkeeping() {
        // This is the bankroll-side half of the capital-short scenario
        // exercised fully in accounting.rs: a reservation that fails leaves
        // the bankroll untouched, so the caller's own requested-stake
        // bookkeeping (not the bankroll) is what carries the "attempted but
        // capital-short" signal forward into `SimulationStats`.
        let mut bankroll = Bankroll::new(&[1_000]).unwrap();
        assert!(!bankroll.reserve(0, 2_000));
        assert_eq!(bankroll.available(0), 1_000);
    }

    /// Any sequence of index/amount operations, replayed against a
    /// venue-by-venue reference ledger that mirrors `Bankroll`'s own
    /// arithmetic, must keep `Bankroll` and the reference in lockstep, and
    /// the running total must equal:
    ///
    /// `initial_total - Σ(settle_loss locked) - Σ(settle_win locked - payout)`
    ///
    /// i.e. money only ever leaves the system via a settlement (loss, or a
    /// win whose payout is less than what was locked against it) — it never
    /// appears or vanishes on `reserve`/`commit_fill`, which only move money
    /// between `available` and `locked` within a venue.
    #[derive(Debug, Clone, Copy)]
    enum Op {
        Reserve {
            venue: usize,
            amount: Cents,
        },
        CommitFill {
            venue: usize,
            filled: Cents,
            unfilled: Cents,
        },
        SettleLoss {
            venue: usize,
            locked: Cents,
        },
        SettleWin {
            venue: usize,
            locked: Cents,
            payout: Cents,
        },
    }

    fn any_op() -> impl Strategy<Value = Op> {
        let venue = 0usize..4;
        prop_oneof![
            (venue.clone(), 0i64..2_000).prop_map(|(venue, amount)| Op::Reserve { venue, amount }),
            (venue.clone(), 0i64..2_000, 0i64..2_000).prop_map(|(venue, filled, unfilled)| {
                Op::CommitFill {
                    venue,
                    filled,
                    unfilled,
                }
            }),
            (venue.clone(), 0i64..2_000)
                .prop_map(|(venue, locked)| Op::SettleLoss { venue, locked }),
            (venue, 0i64..2_000, 0i64..2_500).prop_map(|(venue, locked, payout)| {
                Op::SettleWin {
                    venue,
                    locked,
                    payout,
                }
            }),
        ]
    }

    proptest! {
        #[test]
        fn conservation_holds_across_random_operation_sequences(
            ops in prop::collection::vec(any_op(), 0..200),
        ) {
            let initial = [10_000 as Cents, 5_000, 0, 2_500];
            let bankroll_new = Bankroll::new(&initial).unwrap();
            let mut bankroll = bankroll_new;
            let initial_total: Cents = initial.iter().sum();

            // Reference ledger, updated with exactly the same rules as
            // `Bankroll`, used to cross-check every step (not just the end).
            let mut ref_available = initial;
            let mut ref_locked = [0 as Cents; 4];
            let mut net_settled_out: Cents = 0; // money that left the system

            for op in ops {
                match op {
                    Op::Reserve { venue, amount } => {
                        let ok_ref = amount <= 0 || ref_available[venue] >= amount;
                        let ok = bankroll.reserve(venue as VenueId, amount);
                        prop_assert_eq!(ok, ok_ref);
                        if ok && amount > 0 {
                            ref_available[venue] -= amount;
                            ref_locked[venue] += amount;
                        }
                    }
                    Op::CommitFill {
                        venue,
                        filled,
                        unfilled,
                    } => {
                        bankroll.commit_fill(venue as VenueId, filled, unfilled);
                        let unfilled = unfilled.max(0);
                        let refund = unfilled.min(ref_locked[venue]);
                        ref_locked[venue] -= refund;
                        ref_available[venue] += refund;
                    }
                    Op::SettleLoss { venue, locked } => {
                        bankroll.settle_loss(venue as VenueId, locked);
                        let locked = locked.max(0).min(ref_locked[venue]);
                        ref_locked[venue] -= locked;
                        net_settled_out += locked;
                    }
                    Op::SettleWin {
                        venue,
                        locked,
                        payout,
                    } => {
                        bankroll.settle_win(venue as VenueId, locked, payout);
                        let locked = locked.max(0).min(ref_locked[venue]);
                        ref_locked[venue] -= locked;
                        let credited = payout.max(0); // TRANSFER_FRICTION_BPS is 0
                        ref_available[venue] += credited;
                        net_settled_out += locked - credited;
                    }
                }

                // Every step, not just at the end: Bankroll must match the
                // reference ledger exactly.
                for venue in 0..4 {
                    prop_assert_eq!(bankroll.available(venue as VenueId), ref_available[venue]);
                    prop_assert_eq!(bankroll.locked(venue as VenueId), ref_locked[venue]);
                }
            }

            let total = bankroll.total_available() + bankroll.total_locked();
            prop_assert_eq!(total, initial_total - net_settled_out);
        }
    }
}
