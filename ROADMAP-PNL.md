# ROADMAP-PNL — PnL/ROI Improvement Program

Technical roadmap for raising realized paper-trading PnL/ROI by making the
detector size against what will actually fill, using the depth we already
retain, recovering decayed signals, and accounting for capital honestly.

**Audience:** autonomous development agents. Each workstream below is scoped so
one agent can execute it on one branch without touching files another
concurrent agent owns. Read this whole document before writing code; read
`CLAUDE.md` before committing anything.

---

## 0. Ground truth (why this program exists)

Current baseline (`RESULTS.md`, dated snapshots): **+$15,501.73 (+2.12% ROI)**
from 829 signals, disposition **0 clean fills / 746 partial fills / 83
phantoms**. The mix is structural, not discovered:

| Leak | Evidence |
|---|---|
| Sizing requests full top-of-book depth; queue-survival discount applied only at fill time, after sizing | `crates/arbkit-engine/examples/pipeline.rs:591,604` request 80,000 against Poly depth discounted by `queue_front_run_bps`; sim discount lives in `crates/arbkit-sim/src/latency.rs:103-110` |
| Detection is top-of-book only; 8 retained levels unused | `crates/arbkit-engine/src/aggregator.rs:47-64` reads `book.best()` only; `OutcomeBook::depth_to` (`crates/arbkit-core/src/book.rs:141-147`) has no caller on the detect path |
| Greedy proportional staking floors each leg independently; leftover budget never re-spent | `crates/arbkit-core/src/arb.rs:190-196` |
| One venue per outcome; no line shopping | `aggregator.rs:42-68` picks a single best venue per outcome |
| Price-moved legs discarded outright; no chase/re-quote | `crates/arbkit-sim/src/simulator.rs:357-370` |
| Signals carry no freshness metadata; stale and fresh indistinguishable | `Signal` (`arb.rs:73-90`) has no capture time/TTL |
| Static bankroll; no locked-capital, compounding, or attempted-capital ROI | pipeline budget block, `pipeline.rs:585-604`; `realized_roi_bps` divides by filled stake only (`accounting.rs:187`) |
| Phantom injection is synthetic (`idx % 10 == 0`), tape recorder/player unused for PnL measurement | `pipeline.rs:571-583` |
| Histogram samples only events that emitted a signal | `crates/arbkit-engine/src/engine.rs:150-154` |

## 1. Non-negotiable invariants (apply to every workstream)

From `CLAUDE.md` and the crate docs. A PR that violates any of these is wrong
even if its tests pass.

1. **Hot path purity.** Anything on the feed-update → `Signal` path must not
   allocate, lock, go async, touch `&str`, or make decisions in `f64`. Fixed
   arrays and `i128`/`u64` integer math only. New detection loops must have a
   compile-time bound (documented constant × chunk count).
2. **Pessimistic rounding always.** Payouts floor, effective costs ceil, stakes
   round to tradeable increments without exceeding what the price supports.
   Every reported number is one you can beat, not one you must hit.
3. **Fees before comparison.** No raw-price edge is ever evaluated.
4. **Staleness is out-of-service.** Never interpolate; never trade a stale book.
5. **No arbitrage is not an error.** `Ok(None)` / `None` remains the common
   case; errors are malformed input only.
6. **Detection totality.** The `detection_is_total` property style extends to
   any new entry point: for all inputs, a result — never a panic.
7. **All-or-nothing hedging stays.** A market missing a quote on any outcome
   produces no signal (`aggregator.rs:70-79`). Unhedged/degraded modes are
   explicitly out of scope for this program.
8. Repo conventions: module-level `//!` docs explaining *why*, tests written as
   documentation, `cargo fmt` + `clippy -D warnings` + full test suite green
   before handoff.

## 2. Frozen contracts

These signatures are agreed up front so parallel agents can code against them
before their counterparts land. Changes require updating this section and
notifying dependents in the PR description.

### 2.1 Depth discount (new core module)

```rust
// crates/arbkit-core/src/fill.rs  (new; exported from lib.rs)
/// Share of resting depth expected to survive transit to the venue,
/// in basis points of the raw resting size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepthDiscount { pub survival_bps: u32 }   // 10_000 = untouched

impl DepthDiscount {
    /// Pessimistic (floored) usable depth. 0 when survival_bps is 0.
    #[inline] pub fn discounted(&self, raw_depth: Cents) -> Cents;
    /// Convenience: discount every level of a book slice.
    pub fn discounted_levels(&self, levels: &[Level]) -> arrayvec-like fixed result;
}
```

Semantics identical to `LatencyProfile::effective_depth` (`latency.rs:103-110`)
so detection-side sizing and fill-time checks agree exactly. `arbkit-sim`
refactors `effective_depth` to delegate to this type (single source of truth);
the wire-delay/processing-delay fields stay in `arbkit-sim`.

### 2.2 Level-carrying leg and the new detector (core)

```rust
// crates/arbkit-core/src/arb.rs
pub const MAX_LEVELS_PER_LEG: usize = 8;     // matches book::MAX_LEVELS
pub const MAX_CHUNKS: usize = 16;            // total allocations per signal

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BookLeg {
    pub venue: VenueId,
    pub outcome: OutcomeId,
    pub fee: Fee,
    pub increment: Cents,
    pub levels: [Level; MAX_LEVELS_PER_LEG],  // best-first, pre-discounted by caller
    pub n_levels: u8,                          // 0 => leg unusable
}

pub fn detect_book(legs: &[BookLeg], budget: Cents) -> Result<Option<Signal>>;

/// Legacy signature retained as a thin adapter: builds single-level BookLegs
/// from `Leg`s and calls `detect_book`. Engine migrates off this in B2;
/// removal happens in C1.
pub fn detect(legs: &[Leg], budget: Cents) -> Result<Option<Signal>>;
```

`Signal` changes shape (breaking, approved):

```rust
pub struct Signal {
    allocations: [Allocation; MAX_CHUNKS],
    len: u8,                       // <= MAX_CHUNKS
    pub overround_ppm: u32,
    pub total_stake: Cents,
    pub worst_case_profit: Cents,
    pub profit_bps: u32,
}
// Allocation unchanged: { leg: usize, stake: Cents, payout: Cents }
// `leg` indexes the input BookLeg slice; outcome grouping is recovered by the
// consumer through that slice.
```

> Engine note (B1/B2): ring elements grow by roughly 300 bytes/slot. Verify the
> SPSC ring byte budget where slots are declared in `crates/arbkit-engine/src/ring.rs`
> and reduce slot counts if total footprint regresses beyond reason. Document
> the choice.

### 2.3 Chase policy and TTL (sim)

```rust
// crates/arbkit-sim/src/simulator.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChasePolicy {
    pub enabled: bool,
    pub max_chase_bps: u32,   // max adverse per-leg move vs detected quote, bps of price
}
// SimConfig gains: chase: ChasePolicy, signal_ttl_ns: u64 (0 = infinite)
```

`simulate_with_quotes` keeps its signature; TTL evaluated from
`detection_timestamp_ns` vs the attempt timestamp already modeled.

### 2.4 Maker rebate fee (core)

```rust
// crates/arbkit-core/src/fee.rs
pub enum Fee {
    /* existing variants */
    /// Venue *pays* this many bps of stake for resting liquidity.
    /// Effective price improves: ppm shrinks, floored, saturating at 1 ppm.
    MakerRebateBps(u32),
}
```

Invariant: `effective(MakerRebateBps(r))(q).ppm() <= q.ppm()` for all `q`, and
rebate never produces ppm 0.

### 2.5 Bankroll (sim)

```rust
// crates/arbkit-sim/src/bankroll.rs (new)
pub struct Bankroll { /* fixed [Cents; 32] arrays, no allocation */ }

impl Bankroll {
    pub fn new(initial_per_venue: &[Cents]) -> Result<Self, BankrollError>;
    pub fn available(&self, venue: VenueId) -> Cents;
    /// Reserve attempted stake. false => insufficient balance; caller skips
    /// the trade and records a capital-short disposition.
    pub fn reserve(&mut self, venue: VenueId, amount: Cents) -> bool;
    /// On fill report: filled amount moves to locked (until settlement),
    /// unfilled remainder returns to available.
    pub fn commit_fill(&mut self, venue: VenueId, filled: Cents, unfilled: Cents);
    /// Settlement: winning side pays out into balance; losing side's locked
    /// stake is consumed. Caller applies per-leg.
    pub fn settle_loss(&mut self, venue: VenueId, locked: Cents);
    pub fn settle_win(&mut self, venue: VenueId, locked: Cents, payout: Cents);
    pub fn total_available(&self) -> Cents;
    pub fn total_locked(&self) -> Cents;
}
```

`SimulationStats` (in `accounting.rs`, owned by A5 this program) gains the
disposition funnel: `attempted`, `capital_short`, plus existing counters, and
`attempted_roi_bps()` alongside `realized_roi_bps()`.

---

## 3. Workstreams

Each workstream lists: goal, owned files (exclusive edit rights), forbidden
files, dependencies, spec, required tests, acceptance criteria.

### Wave 1 — fully parallel, no inter-dependencies

#### A1 — Core depth-discount primitive

- **Owns:** `crates/arbkit-core/src/fill.rs` (new), one `pub mod` line +
  re-export in `crates/arbkit-core/src/lib.rs`.
- **Spec:** implement contract 2.1. Integer math mirroring
  `latency.rs:103-110` exactly (same floored results for same inputs — add a
  cross-check test once A4a lands; until then assert against hand-computed
  values). `discounted_levels` returns a fixed-size array + len, no alloc.
- **Tests:** unit table (0, partial, 10_000 bps; zero/negative depth);
  equivalence property vs a reference implementation copied into the test.
- **Done when:** `DepthDiscount` exported; clippy/fmt/tests green.

#### A2 — `detect_book`: level-walking detection with target-payout search

The heart of the program. Owns `crates/arbkit-core/src/arb.rs`,
`crates/arbkit-core/tests/properties.rs`.

- **Model.** Input chunks are `(BookLeg, level_index)` pairs flattened from the
  input slice (n ≤ MAX_CHUNKS after truncation; truncation must keep ≥1 level
  per distinct outcome present in input). Chunk j has effective price
  `p_j = fee.effective(level.price)` (fees first, invariant 3), increment
  `g_j`, capacity `K_j` = level size **as passed** (caller pre-discounts; see
  B1). Chunks group by `outcome`.
- **Plan and objective.** A plan is integer stakes `s_j = m_j·g_j`, `m_j ≥ 1`,
  `s_j ≤ K_j`, `Σs_j ≤ budget`. If outcome o wins, its payout is
  `P_o = Σ_{j∈o} floor(s_j·PPM / p_j)` (per-chunk floored payouts summed).
  Guaranteed profit `Π = min_o P_o − Σ_j s_j`. Maximize Π.
- **Algorithm (bounded, allocation-free).** Binary-search the guaranteed
  payout target T over `[0, T_hi]`, `T_hi = budget·PPM / min_j(p_j)`:
  - Feasibility(T): per outcome group, distribute T across its chunks
    proportional to `p_j`; each chunk's raw need rounds **up** to the next
    increment multiple (we are sizing toward a payout floor, so rounding up
    the stake is safe — cost is checked afterwards); run a bounded repair
    loop (≤ number of chunks in group iterations) bumping the chunk with the
    best marginal payout-per-cent by one increment until the group's summed
    floored payout ≥ T or the bound trips (⇒ infeasible). Reject T if any
    chunk exceeds `K_j` or total exceeds budget.
  - ~40 binary-search iterations × ≤16 chunk repairs; all `i128`.
  - At optimum T\*, recompute stakes and payouts exactly and pessimistically;
    build `Signal` only if `Π > 0`. Reported `worst_case_profit` must equal
    the recomputed minimum, not T\*.
- **Legacy:** reimplement `detect` as the single-level adapter (contract 2.2).
  Existing tests in `arb.rs` must pass unchanged — they encode the pessimism
  contract (see the `$99,999 / 2,040¢` test at `arb.rs:251-269`). Where the
  new search beats greedy flooring, old assertions may need *better* values;
  improving an asserted number requires a comment proving why the new value is
  the true pessimistic optimum.
- **Required tests:**
  - Improvement monotonicity: for identical inputs, `Π_detect_book ≥
    Π_old_greedy` — provable because the old greedy plan is a member of the
    new feasible set. Property-test over generated inputs.
  - Extend `detection_is_total` to `detect_book` (fuzzed BookLegs incl. n_levels=0,
    duplicate outcomes, capacity < increment).
  - Never-overstate: recomputed per-outcome payouts − total_stake ≥ reported
    profit for every allocation, mirroring `arb.rs:266-268`.
  - Multi-level case beats top-of-book-only case (thin top, deep L2).
  - Fee variants interact correctly with per-level effective prices
    (`CommissionBps`, `StakeFeeBps`, and `MakerRebateBps` once A3 lands —
    gate behind cfg if A3 hasn't merged).
- **Done when:** all above green; `cargo run --example pipeline --release`
  still runs (engine compiles against adapted `detect`) with unchanged-or-
  better printed sample PnL.

#### A3 — Fee precision: maker rebates (core) + exact Kalshi settlement fee (sim)

- **Owns:** `crates/arbkit-core/src/fee.rs` (+tests therein),
  `crates/arbkit-sim/src/order.rs` (+its fill-fee application point).
- **Maker rebate:** implement contract 2.4. Update every exhaustive `match` on
  `Fee` (grep `Fee::` workspace-wide; expect `fee.rs`, possibly feed/engine
  config plumbing — coordinate any file outside `fee.rs`/`order.rs` in the PR
  description rather than editing silently).
- **Exact Kalshi fee.** Continuous `kalshi_stake_fee_bps` stays authoritative
  for *detection* (it floors the real charge — pessimistic). At *fill
  accounting*, apply the published per-order charge exactly: contracts
  `C = stake_cents / 100` (Kalshi contracts are $1), fee in cents

  ```text
  fee_cents = ceil( C · p_ppm · (PPM − p_ppm) · 7 / 1_000_000_000_000 )
  ```

  (check: C=100 @ 50¢ → 175¢ = the published $1.75/100-contract ceiling).
  Apply in `order.rs`'s fill-cost computation; keep `div_ceil` in u128.
- **Tests:** rebate monotonicity property (never worsens, never reaches 0 ppm);
  exact-fee table including the ceiling case and cheap-contract cases from
  `fee.rs:159-166`; property that accounted fee ≥ continuous-form estimate
  per order.
- **Done when:** green suite; no behavioral change for existing `Fee` users.

#### A4a — Chase/re-quote policy (sim only)

- **Owns:** `crates/arbkit-sim/src/simulator.rs`, `crates/arbkit-sim/src/lib.rs`
  (export), sim tests.
- **Spec:** implement contract 2.3. Today a leg whose best price moved past the
  detected quote is dropped (`simulator.rs:357-370`). When `chase.enabled`:
  1. TTL gate: `attempt_ts − detection_timestamp_ns > signal_ttl_ns` ⇒ keep
     current behavior (unfilled/phantom).
  2. Joint re-check at arrival: recompute fee-adjusted overround using the
     arrival prices already supplied to `simulate_with_quotes` for **all**
     legs. Require `arrival_overround_ppm < PPM` **and** every leg's adverse
     move vs its detected quote ≤ `max_chase_bps`. One-leg-only chasing is
     forbidden (directional risk).
  3. On pass: fill all legs at arrival price against queue-discounted arrival
     depth (existing `evaluate_leg` mechanics, expected = arrival quote);
     classify `ChasedFill`; `ExecutionPnl` computes from actual fills.
  4. Stats: add `chased_count`, `chased_profit_cents` to `SimulationStats`
     **via a small patch coordinated with A5** if accounting.rs conflicts —
     prefer landing after A5 or coordinating in PR descriptions.
- **Tests:** chased trade profitable at arrival but not beyond what arrival
  prices support (invariant: chase never reports more than joint arrival-edge);
  TTL expiry; max_chase_bps boundary; disabled-policy regression equals current
  behavior.
- **Done when:** green; phantom count on default pipeline config unchanged
  (chase defaults off).

#### A5 — Bankroll and disposition funnel (sim)

- **Owns:** `crates/arbkit-sim/src/bankroll.rs` (new),
  `crates/arbkit-sim/src/accounting.rs`, `crates/arbkit-sim/src/error.rs` (new
  error variants), `crates/arbkit-sim/src/lib.rs` exports, sim tests.
- **Spec:** implement contract 2.5. Extend `SimulationStats.record` with the
  funnel fields (attempted, capital_short) and `attempted_roi_bps()`;
  bump the JSON report schema minor version (coordinate final schema shape
  with C1, who consumes it).
- **Conservation property test:** across randomized reserve/commit/settle
  sequences: `Σ available + Σ locked == initial − settled losses − transfer/
  fee costs` (transfer friction is a later flag; keep the hook, default off).
- **Done when:** green; nothing else in sim changes behavior.

#### A6 — Honest latency measurement (engine)

- **Owns:** `crates/arbkit-engine/src/engine.rs` (histogram call sites +
  `EngineStats`), engine tests.
- **Spec:** record per-event service time for **every** processed event
  (move/clone the record call out of the `Ok(Some(signal))` branch at
  `engine.rs:150-154`). Keep a separate counter of signal-emitting events so
  the old metric remains derivable. Document in `histogram.rs` module docs
  that percentile semantics changed (all-events, not signal-hits).
- **Done when:** green; pipeline output labels the metric unambiguously.

### Wave 2 — parallel after named Wave 1 merges

#### B1 — Execution-aware sizing wiring (engine) — *after A1, A2*

- **Owns:** `crates/arbkit-engine/src/aggregator.rs`,
  `crates/arbkit-engine/src/event.rs` (config types),
  `crates/arbkit-engine/src/engine.rs` (emission cooldown only — coordinate
  with A6's merged state), engine tests.
- **Spec:**
  - `MarketConfig` gains `venue_survival_bps: [u32; MAX_VENUES]` (default
    10_000). Aggregator passes each level through
    `DepthDiscount { survival_bps }.discounted(..)` before constructing legs —
    detection now sizes against transit-surviving depth
    (`aggregator.rs:61` is the current unchecked capacity).
  - Migrate aggregation onto `detect_book`/`BookLeg` (contract 2.2), initially
    still one level per outcome (multi-level collection is B2).
  - Signal dedup/cooldown: `MarketConfig.signal_cooldown_ticks: u64` (default 0);
    slab entry stores last-emit tick per market; suppress duplicate emission
    within the window. No allocation (counter in the existing slab entry).
- **Tests:** discounted sizing shrinks requested stake to survive the sim's
  fill model (end-to-end assertion moves clean-fill rate above 0 on the
  pipeline workload once C1 wires profiles — unit-test the aggregator math
  directly here); cooldown suppression unit test.
- **Done when:** green; pipeline runs; sample PnL improves or clean-fill count
  rises.

#### B2 — Multi-venue line shopping (engine) — *after B1 (branches off it)*

- **Owns:** `crates/arbkit-engine/src/aggregator.rs` (continuation),
  engine tests.
- **Spec:** replace single-best selection (`aggregator.rs:42-68`) with
  per-outcome collection of the best K levels across **all** venues, ordered
  by fee-adjusted effective ppm ascending. Global cap MAX_CHUNKS with fair
  coverage: first pass takes the single best level per outcome, remaining
  budget filled in global best-effective order. Respect staleness
  (`best()/levels()` already yield nothing when stale). Keep the
  all-or-nothing rule (invariant 7).
- **Tests:** two-venue split backing one outcome produces a signal where
  either venue alone fails; chunk cap respected; outcome coverage guaranteed
  under adversarial ordering.
- **Done when:** green; property: aggregate stake per outcome never exceeds
  combined discounted capacity.

#### B3 — Sim TTL enforcement + chased-stat wiring — *after A4a and A5*

- **Owns:** `crates/arbkit-sim/src/simulator.rs`, `accounting.rs` (small),
  sim tests.
- **Spec:** wire `signal_ttl_ns` gating from SimConfig into the attempt path;
  fold A4a's chased counters into A5's funnel struct if they landed
  separately; expose `funnel()` accessor returning the disposition summary
  consumed by C1's report.
- **Done when:** green; JSON report includes funnel.

### Wave 3 — sequential integration (single agent)

#### C1 — Pipeline integration, tape mode, honest reporting

- **Owns:** `crates/arbkit-engine/examples/pipeline.rs`, `README.md`
  (metrics table only), `RESULTS.md`, dashboard data via
  `npm --prefix dashboard run record`.
- **Spec:**
  1. `--tape PATH` flag: replay recorded tapes through
     `arbkit_feed::tape::TapePlayer` into the engine (player exists:
     `crates/arbkit-feed/src/tape/player.rs`). Synthetic generator stays the
     default when the flag is absent.
  2. Replace `idx % 10 == 0` phantom injection (`pipeline.rs:571-583`) with
     volatility-derived decay: track a running adverse-move rate per market
     during replay (EWMA of quote regressions within the modeled transit
     window); arrival prices sampled accordingly. Synthetic mode may keep the
     scripted behavior, labeled as such.
  3. Wire A5 `Bankroll` into the simulation loop: per-trade budget derived
     from `total_available()` (compounding), reserves/commits/settlements per
     execution report; static-budget path kept behind a flag for comparison.
  4. Report: print and emit the disposition funnel (attempted → capital-short
     → chased → clean → partial → phantom → broken), dual ROI
     (`realized_roi_bps` and `attempted_roi_bps`), chased recovery PnL,
     capital utilization. Update the JSON schema consistently with A5/B3.
  5. Remove the legacy `detect` adapter if B1/B2 left no callers (approved
     breaking change); otherwise mark `#[deprecated]` with a pointer to
     `detect_book`.
  6. Record a new dated baseline column in `RESULTS.md` (preserve prior hosts'
     numbers per repo policy), refresh the README metrics table, run the
     dashboard recorder, review generated snapshot before committing.
- **Done when:** end-to-end release run shows (a) clean fills > 0, (b) chased
  recoveries counted, (c) attempted vs realized ROI both reported, (d) all
  four repo checks green.

---

## 4. Schedule, ownership matrix, conflict rules

```
Wave 1 (parallel):  A1  A2  A3  A4a  A5  A6
Wave 2 (parallel):  B1 (needs A1,A2)   B3 (needs A4a,A5)
                    B2 branches off B1
Wave 3 (serial):    C1 (needs all)
```

| File | Exclusive owner |
|---|---|
| `core/src/fill.rs` (new) | A1 |
| `core/src/arb.rs`, `core/tests/properties.rs` | A2 |
| `core/src/fee.rs` | A3 |
| `sim/src/simulator.rs` | A4a, then B3 |
| `sim/src/accounting.rs`, `bankroll.rs`, `error.rs` | A5, then B3 (small patches) |
| `sim/src/order.rs`, `latency.rs` (delegate refactor) | A3 (order.rs), A1 (latency delegate) |
| `engine/src/engine.rs` | A6, then B1 (cooldown) |
| `engine/src/aggregator.rs`, `event.rs` | B1, then B2 |
| `examples/pipeline.rs`, `README.md`, `RESULTS.md`, dashboard | C1 |
| `*_src/lib.rs` export lines | owner adds one line; trivial rebase acceptable |

Rules:

- Do not edit files outside your ownership. Cross-cutting needs (an extra
  `Fee` match arm outside `fee.rs`, an export line) go in the PR description;
  the dependent-wave agent resolves them.
- Branches: `ws/a1-depth-discount`, `ws/a2-detect-book`, … Rebase onto main at
  start of your wave, not before.
- If your dependency hasn't merged when you finish, gate the affected code
  behind a `cfg(feature = "...")` stub or land against the frozen contract
  with the interface compiled but inert — never invent a different signature.

## 5. Definition of done (every workstream)

```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run --example pipeline --release        # runs; no regression in printed p99 budget verdict
```

Plus, for hot-path work (A2, B1, B2): re-run the release pipeline and confirm
hot-loop p99 stays in the same magnitude band as the latest dated baseline
(budget is 50 µs; measured headroom is ~500×; any regression beyond 2× the
recorded p99 needs explanation in the PR).

For C1 additionally: new dated RESULTS.md column, dashboard snapshot reviewed,
README table updated, prior baselines untouched.

## 6. Risk register

| Risk | Mitigation |
|---|---|
| Target-payout search subtly overstates profit (rounding direction flipped somewhere) | Invariant test "recomputed payouts from allocations ≥ reported"; property fuzz; review of every `div`/`ceil` in A2 by a second agent before merge |
| Growing `Signal` blows ring memory/cache assumptions | B1 verifies `ring.rs` budgets; cap MAX_CHUNKS=16; measure before raising |
| Chase logic manufactures profit from stale info | Joint arrival-price recheck mandatory; TTL default finite; chase off by default |
| Wave-2 agents blocked on slow Wave-1 merges | Frozen contracts (§2) allow coding ahead; feature-gate fallback defined above |
| Metric semantics shift confuses baseline comparisons | A6/C1 label changed percentiles; RESULTS keeps old columns intact |
