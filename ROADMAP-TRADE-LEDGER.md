# ROADMAP-TRADE-LEDGER — Per-Trade Accuracy Ledger Program

Technical roadmap for a "proof of trading accuracy" viewer: every detected
arbitrage signal persisted alongside its simulated outcome, rendered in the
dashboard so users can see expected-vs-realized PnL at trade granularity
instead of trusting aggregate counters.

**Audience:** autonomous development agents. Each workstream below is scoped so
one agent can execute it on one branch without touching files another
concurrent agent owns. Read this whole document before writing code; read
`CLAUDE.md` before committing anything.

---

## 0. Ground truth

What exists today:

| Asset | Where |
|---|---|
| Per-signal ring record (`SignalEvent`: market id, latency, timestamps) | `crates/arbkit-engine/src/event.rs:12-23`, slot at `event.rs:244-257` |
| Detection payload (`Signal`: profit_bps, total_stake, worst_case_profit, allocations) | `crates/arbkit-core/src/arb.rs:189-208` |
| Per-execution outcome (`ExecutionReport`: classification, per-leg `LegFillResult`, `ExecutionPnl` with expected vs realized vs slippage) | `crates/arbkit-sim/src/simulator.rs:72-87`, `order.rs:119-140`, `accounting.rs:15-34` |
| Classification / phantom reasons | `crates/arbkit-sim/src/phantom.rs:16,47` |
| Run report JSON consumed by the dashboard (`PipelineReport`, schemaVersion 1) | `crates/arbkit-engine/examples/pipeline.rs:23-117`; zod mirror in `dashboard/src/data/schema.ts:16-121` |
| Dashboard (React 19 + Vite + recharts + zod, Cloudflare Workers deploy), run picker, lazy chart sections | `dashboard/src/App.tsx`, `dashboard/src/components/` |

What is missing:

1. **No persistent per-trade log.** The simulator returns an `ExecutionReport`
   per call but nothing stores them; the JSON report carries aggregates plus a
   single sample signal (`pipeline.rs` detection section).
2. **No trade-level UI.** The dashboard shows run-level stats only.
3. Signals reference interned ids; human-readable names must be resolved at
   the example boundary via `arbkit-match` registries (the example already
   builds them).

## 1. Non-negotiable invariants (apply to every workstream)

From `CLAUDE.md`. A PR that violates any of these is wrong even if its tests pass.

1. **Hot path untouched.** All ledger capture happens in the example's main
   thread after signals are consumed off the ring. No engine, core, sim, feed,
   or match source changes are required by this program; none are permitted
   except where explicitly owned below.
2. **Pessimistic numbers travel intact.** Money values are integer cents,
   rates are integer bps/ppm exactly as computed upstream. No rounding,
   no floats anywhere in the chain — including TypeScript display math
   (format only; never recompute profit).
3. **No fabrication.** If a run has no trade log, the UI says so. Never
   synthesize rows to fill gaps.
4. **Schema-versioned everything.** New files carry `schemaVersion`; loaders
   validate with zod and fail loudly on mismatch.
5. Repo conventions: module-level docs explaining *why*, tests as
   documentation, `cargo fmt` + `clippy -D warnings` + full test suite green
   before handoff (Rust streams); `npm test && npm run build` green
   (dashboard streams).

## 2. Frozen contracts

Agreed up front so parallel agents can code against them before counterparts
land. Changes require updating this section and notifying dependents in the PR
description.

### 2.1 Trades file location and transport

```
pipeline flag:      --trades <path>        (default: sibling of --json path with .trades.jsonl suffix)
recorded artifact:  dashboard/public/data/runs/<run-id>.trades.jsonl
index entry:        index.json runs[i] gains optional "tradesFile": "<filename>"
                    (absent => pre-ledger run; consumers must handle absence)
```

### 2.2 Trades JSONL format

Line 1 — header object. Lines 2..n — one trade each. UTF-8, `\n` delimited,
camelCase keys matching the existing `PipelineReport` convention.

```jsonc
// Line 1
{
  "schemaVersion": 1,
  "kind": "arbkit-trades",
  "runId": "<matches --json run.id>",
  "tradeCount": <whole>,
  "recordedAtEpochMs": <whole>,          // optional
}
```

```jsonc
// Lines 2..n — TradeRecord
{
  "seq": <whole>,                        // 0-based, dense
  "detectionTimestampNs": <whole>,
  "latencyNs": <whole>,                  // engine-measured service latency
  "marketLabel": "<human-readable>",     // resolved via arbkit-match registry
  "edgeBps": <whole>,                    // Signal::profit_bps
  "overroundPpm": <whole>,
  "requestedStakeCents": <int>,          // Signal::total_stake
  "expectedProfitCents": <int>,          // ExecutionPnl::expected_profit (fee-adjusted detection view)
  "worstCaseProfitCents": <int>,         // Signal::worst_case_profit
  "realizedProfitCents": <int>,          // ExecutionPnl::realized_profit (may be negative)
  "slippageCents": <int>,                // ExecutionPnl::slippage
  "feesPaidCents": <int>,                // ExecutionPnl::total_fees
  "fillRatioBps": <whole>,               // ExecutionPnl::fill_ratio_bps
  "classification": "clean" | "proportional" | "phantom" | "brokenLeg",
  "chased": <boolean>,
  "legs": [                              // <= MAX_LEGS (4); omit empty legs
    {
      "venueLabel": "<human-readable>",
      "outcomeLabel": "<human-readable>",
      "status": "filled"
             | { "partiallyFilled": { "filledCents": <int>, "unfilledCents": <int>, "reason": "<string>" } }
             | { "unfilled": "<string>" },
      "requestedStakeCents": <int>,
      "filledStakeCents": <int>,
      "netPayoutCents": <int>
    }
  ]
}
```

Rules:
- All money fields integer cents; all rate fields integer bps/ppm. No floats,
  ever (invariant 2).
- `classification` values map 1:1 from `ArbExecutionClassification`
  (`phantom.rs:47`); phantom reason strings come from `PhantomReason`.
- Labels resolved once at startup through the example's existing
  `CanonicalRegistry`/`VenueRegistry` lookups; the string never crosses into
  engine internals.
- Header `tradeCount` must equal the number of subsequent lines (validation
  target).

### 2.3 Dashboard loader signature

```ts
// dashboard/src/data/schema.ts (owned by D1)
export const TradeLogHeaderSchema = /* §2.2 header */;
export const TradeRecordSchema    = /* §2.2 record */;
export type TradeRecord = z.infer<typeof TradeRecordSchema>;

// dashboard/src/data/trades.ts (owned by D1)
export async function loadTradeLog(run: RunSnapshot): Promise<TradeLog | null>;
// null when index entry lacks tradesFile or fetch/parse fails validation.
export type TradeLog = { header: TradeLogHeader; records: TradeRecord[] };

// dashboard/src/data/tradeMetrics.ts (owned by D1)
export function tradeMetrics(records: TradeRecord[]): {
  hitRateBps: number;          // share of records with realizedProfitCents > 0
  totalExpectedCents: number;
  totalRealizedCents: number;
  medianSlippageCents: number;
  phantomRateBps: number;      // phantom / total
  cleanShareBps: number;       // clean / total
};
// All pure functions over validated data. Integer-safe: sums over cents fit
// safely in JS number range for realistic trade counts; assert count < 2^31.
```

---

## 3. Workstreams

Each workstream lists: goal, owned files (exclusive edit rights), forbidden
files, dependencies, spec, required tests, acceptance criteria.

### Wave 1 — fully parallel, no inter-dependencies

#### L1 — Rust: per-trade ledger emission in the pipeline example

- **Owns:** `crates/arbkit-engine/examples/pipeline.rs`, new example-local
  module(s) under `crates/arbkit-engine/examples/` if split for size (e.g.
  `examples/trades_ledger.rs` included via `#[path]`), dev-dep additions to
  `crates/arbkit-engine/Cargo.toml` **only if** unavoidable (prefer none —
  `serde_json` is already a dev-dep).
- **Forbidden:** everything under `crates/*/src/`, `dashboard/`.
- **Dependencies:** none.
- **Spec:**
  1. Define `TradeRecord` per contract §2.2 inside the example. Pair each
     collected `SignalEvent` with its `ExecutionReport` at the point where the
     simulator is already invoked per signal (see the simulator section of
     `pipeline.rs`). Sequence numbers dense from 0.
  2. Resolve labels via the registries the example already constructs; fall
     back to `"market:<id>"` / `"venue:<id>"` strings rather than panicking if
     a lookup misses (ledger must stay total).
  3. `--trades <path>` flag; default path derived from `--json` path when both
     given, else `trades.jsonl` in cwd. Write header first, then one serde
     struct per line. Flush and check the `Result` — a failed ledger write is
     reported to stderr but does not abort the run after trading completes.
  4. Print a one-line summary in the existing output style: trades written,
     hit count, realized PnL total.
  5. Keep hot-path sections byte-identical in behavior (no timing changes);
     ledger capture happens post-consumption only.
- **Tests:** the example has no test harness today — add a small unit-testable
  core: a pure `build_trade_record(signal_event, report, labels) ->
  TradeRecord` function plus round-trip test `serde_json::to_string` → parse →
  equality, placed as `#[cfg(test)]` in the example module (dev-profile tests
  of examples run under `cargo test --workspace -p arbkit-engine`). Property:
  emitted cents/bps equal inputs exactly (invariant 2).
- **Done when:** `cargo fmt/clippy/test` green;
  `cargo run --example pipeline --release -- --json /tmp/r.json --trades
  /tmp/t.jsonl` produces a valid file whose header count matches line count
  and whose aggregates reconcile with the printed simulation section.

#### D1 — Dashboard: schema, loader, accuracy metrics

- **Owns:** `dashboard/src/data/schema.ts` (append), 
  `dashboard/src/data/trades.ts` (new), `dashboard/src/data/tradeMetrics.ts`
  (new), `dashboard/src/data/tradeMetrics.test.ts` (new),
  `dashboard/scripts/record-run.mjs` (trades-file copy + index field).
- **Forbidden:** `App.tsx`, `components/`, `styles.css`, Rust crates.
- **Dependencies:** none (codes against contract §2.2/§2.3).
- **Spec:**
  1. Zod schemas per §2.3 mirroring §2.2 exactly; discriminated unions for leg
     status and classification.
  2. Extend `RunIndexSchema` entries with optional `tradesFile: string`.
  3. `loadTradeLog` per §2.3: fetch `data/runs/<tradesFile>`, split lines,
     validate header then each record; return `null` on absence (404),
     throw on schema violation (callers decide presentation). Cap accepted
     file size defensively (e.g. reject > 20 MB) with a clear error.
  4. `tradeMetrics` per §2.3. Median over integer cents: average of two middle
     values may halve — return floored cents and document.
  5. Update `record-run.mjs`: accept the trades path produced by the pipeline
     run, copy to `public/data/runs/<id>.trades.jsonl`, add `tradesFile` to
     the index entry, validate header line count against `tradeCount` before
     recording (fail the recording script loudly on mismatch).
- **Tests:** Vitest — schema accepts fixture lines (embed a valid sample in
  the test), rejects wrong `schemaVersion`, float money, unknown
  classification, header/count mismatch; loader returns `null` on fetch 404
  (mock); metric functions against hand-computed fixtures including empty
  records array (must not divide by zero).
- **Done when:** `npm test && npm run build` green; fixtures cover every
  classification and leg-status variant.

### Wave 2 — parallel after Wave 1 merges

#### D2 — Dashboard: TradeLedger component

- **Owns:** `dashboard/src/components/TradeLedger.tsx` (new),
  `dashboard/src/components/TradeLedger.test.tsx` (new).
- **Forbidden:** `App.tsx`, `styles.css`, data layer files.
- **Dependencies:** D1 merged (or coded against §2.3 exports behind a local
  stub that is deleted before merge).
- **Spec:**
  1. Props: `{ log: TradeLog | null; error?: string }`. Render three zones:
     summary cards, chart, table.
  2. Summary cards (reuse existing card/table CSS vocabulary from
     `Charts.tsx`/`BudgetRuler.tsx` markup patterns): hit rate, expected vs
     realized total, median slippage, phantom rate, clean share. Values from
     `tradeMetrics` only — no recomputation in JSX.
  3. Chart (lazy recharts pattern like `Charts.tsx`): per-trade expected vs
     realized bars or scatter; diagonal reference makes slippage losses
     visually obvious below the line.
  4. Table: columns seq/time, edge bps, stake, expected, realized, Δ,
     classification badge, chased marker; sortable by edge/realized/Δ;
     filter chips per classification; "profitable only" toggle. Row expansion
     shows per-leg audit (venue, outcome, status incl. partial-fill reasons,
     stakes, net payout). Paginate at 200 rows/page (simple slice state —
     no new virtualization dependency unless row counts demand it; justify in
     PR if added).
  5. States: `log === null` → "No trade log recorded for this run" honest
     empty state (invariant 3); error → message with retry hint; zero trades
     → explicit "detector found no arbitrage in this run" (not an error).
  6. Accessibility: table semantics (`<table>`, scoped headers),
     `aria-live` on filter result counts, keyboard-operable expanders.
- **Tests:** Vitest/RTL — renders cards matching `tradeMetrics` fixture;
   filter/sort behavior; expansion reveals leg statuses; empty/error states.
- **Done when:** `npm test && npm run build` green; no console warnings.

#### D3 — Dashboard: App integration and styling

- **Owns:** `dashboard/src/App.tsx` (new section + nav link + loader wiring),
  `dashboard/src/styles.css` (ledger styles appended).
- **Forbidden:** `components/TradeLedger.tsx` contents, data layer files.
- **Dependencies:** D1 merged; codes against D2's prop contract
  `{ log, error }` (frozen here so D2/D3 proceed in parallel).
- **Spec:**
  1. Lazy-load `TradeLedger` like existing charts; new nav anchor `#trades`
     labelled consistently with existing section voice ("Execution",
     "Verification" → suggest "Trades").
  2. Wire `loadTradeLog(selected)` into the existing selected-run effect
     lifecycle; reset trade-log state when the selected run changes; do not
     block the rest of the page on trade-log failure.
  3. Styles follow the existing evidence-ledger aesthetic (see binder-rail,
     hero-facts patterns in `App.tsx:92-140`); dark/light consistent with
     current tokens in `styles.css`.
- **Done when:** `npm test && npm run build` green; switching runs swaps the
  ledger correctly; a pre-ledger run shows the honest empty state.

### Wave 3 — sequential integration (single agent)

#### V1 — Data generation, recording, end-to-end verification

- **Owns:** fresh generated artifacts only:
  `dashboard/public/data/runs/*` (new dated snapshot + trades file +
  `index.json` update via recorder), `RESULTS.md` (dated baseline column for
  the ledger-enabled run, prior baselines untouched), README dashboard notes
  if the recorder usage changed.
- **Forbidden:** all source files.
- **Dependencies:** L1, D1–D3 merged.
- **Spec:**
  1. `cargo run --release --example pipeline -- --json … --trades …`;
     sanity-check reconciliation (§ L1 done-when) before recording.
  2. Record via `record-run.mjs`; verify `index.json` gains `tradesFile`.
  3. Full four-check suite (below) plus `npm test/build` in `dashboard/`;
     manually review the deployed preview: cards reconcile with the JSONL,
     filters work, empty state appears on an old run.
  4. Add dated `RESULTS.md` column noting ledger capture; preserve prior
     hosts' numbers per repo policy.
- **Done when:** all checks green; a user opening the dashboard sees a
  complete, reconciled proof ledger for the new run.

## 4. Schedule, ownership matrix, conflict rules

```
Wave 1 (parallel):  L1            D1
Wave 2 (parallel):  D2 (needs D1) D3 (needs D1; codes to D2 prop contract)
Wave 3 (serial):    V1 (needs all)
```

| File | Exclusive owner |
|---|---|
| `engine/examples/pipeline.rs` (+ example-local modules, Cargo.toml dev-deps if unavoidable) | L1 |
| `dashboard/src/data/schema.ts`, `trades.ts` (new), `tradeMetrics.ts` (new), `scripts/record-run.mjs` | D1 |
| `dashboard/src/components/TradeLedger.tsx` (+test) | D2 |
| `dashboard/src/App.tsx`, `dashboard/src/styles.css` | D3 |
| `dashboard/public/data/runs/*`, `RESULTS.md`, README notes | V1 |

Rules:

- Do not edit files outside your ownership. Cross-cutting needs go in the PR
  description; the dependent-wave agent resolves them.
- Branches: `ws/l1-trades-jsonl`, `ws/d1-schema-loader`,
  `ws/d2-ledger-component`, `ws/d3-app-integration`, `ws/v1-data-refresh`.
  Rebase onto main at the start of your wave, not before.
- If your dependency hasn't merged when you finish, code against the frozen
  contract (§2) with the interface compiled but inert — never invent a
  different signature or shape.

## 5. Definition of done (every workstream)

Rust streams (L1):

```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run --example pipeline --release        # unchanged p99 budget verdict
```

Dashboard streams (D1–D3):

```
npm --prefix dashboard test
npm --prefix dashboard run build
```

V1 additionally: full Rust + dashboard suites, recorded snapshot reviewed,
prior baselines untouched.

## 6. Risk register

| Risk | Mitigation |
|---|---|
| Ledger drift: JSONL aggregates disagree with printed run report | Reconciliation assertion required in L1 done-when; recorder validates header count (D1 step 5) |
| Two agents reshape §2.2 independently | Contract frozen here; any change requires editing this section first and noting dependents in the PR |
| Large trade logs bloat the Workers bundle/deploy | Files live in `public/` static assets, fetched lazily; D1 enforces a size cap; pagination in D2 |
| Float leakage into displayed money | Invariant 2 + D1 tests rejecting float money fields; components format integers only |
| Pre-ledger runs break the new section | Absence-typed loader (`null`), honest empty state (invariant 3), covered in D3/V1 review |
| Example grows unwieldy | L1 may split via example-local module; no crate source changes permitted (invariant 1) |
