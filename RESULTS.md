# `arbkit`: Benchmark & Pipeline Execution Results

**Version:** 0.1.0  
**Repository:** `https://github.com/harlanljones/arbkit`  
**Live Interactive Dashboard:** `https://arbkit.harlanljones.com/`  
**Test Dates:** August 19, 2026 (baseline); August 21, 2026 (Linux x86_64, earlier revision); August 21, 2026 (commit `f9623ab`); August 22, 2026 (ledger-enabled run, commit `0b0306e`); August 22, 2026 (B1/B2/C1 execution-aware detection run — see provenance note below)  
**Toolchain:** `rustc 1.97.1` (`aarch64-apple-darwin` baseline; `x86_64-unknown-linux-gnu` current)  
**Build Profile:** `release` (`lto = "fat"`, `codegen-units = 1`, `panic = "abort"`)  

---

## 1. Executive Summary

This report documents the live end-to-end pipeline execution results of `arbkit` across market data ingestion, single-threaded hot loop detection, sub-microsecond latency measurement, and execution simulation. All benchmark distributions and financial ledgers can be interactively explored in the live web application at **[arbkit.harlanljones.com](https://arbkit.harlanljones.com/)**.

The August 21 columns are two separate measurements on the **same** Linux
i7-14700K host, taken at different points in the codebase's history — the
"earlier revision" column is the previously-recorded run, preserved as a
dated baseline per repo policy; the "current" column is this session's run
against commit `f9623ab` (working tree clean). The 829-signal detection
sequence, sample edge, and fill accounting shifted between the two because
the detector/simulator logic changed, not because of host or workload
differences — the same synthetic stream, replayed against today's code,
deterministically reproduces the "current" figures below.

| Metric | Apple Silicon baseline (200k ticks) | Linux x86_64, earlier revision (200k ticks) | Linux i7-14700K, `f9623ab` (2M ticks) | Linux i7-14700K, ledger run — `0b0306e` (2M ticks) | Linux i7-14700K, B1/B2/C1 run (2M ticks) |
|---|---:|---:|---:|---:|---:|
| Ingestion Throughput | 3,533,782 msg/sec | 6,347,554 msg/sec | 7,721,309 msg/sec | 7,629,443 msg/sec | 2,809,397 msg/sec |
| Hot Loop Latency (p99) | 0.250 µs (250 ns) | 0.100 µs (100 ns) | 0.080 µs (80 ns) | 0.100 µs (100 ns) | 0.320 µs (320 ns) |
| Hot Loop Latency (Median / p50) | 0.200 µs (200 ns) | 0.090 µs (90 ns) | 0.050 µs (50 ns) | 0.050 µs (50 ns) | 0.280 µs (280 ns) |
| Measured Phantom Rate | 10.01% (1,001 bps) | 10.01% (1,001 bps) | 10.01% (1,001 bps) | 10.01% (1,001 bps) | 10.01% (1,001 bps) |
| Clean Fills / Simulated Signals | 0 / 829 | 0 / 829 | 0 / 829 | 0 / 829 | **746 / 829** |
| Paper-Trading Realized PnL | +$15,501.73 | +$15,501.73 | +$15,706.38 | +$15,706.38 | **+$21,491.58** |
| Realized Settlement ROI | +2.12% | +2.12% | +2.15% | +2.15% | **+2.94%** |
| Workspace Test Verification | 114 / 114 passed | 114 / 114 passed | 159 / 159 passed | 159 / 159 passed | 170 / 170 passed |

> **Provenance note on the B1/B2/C1 column:** the snapshot filename carries
> commit `760946e` because the pipeline stamps `git rev-parse HEAD`, but this
> run includes the uncommitted ROADMAP-PNL B1/B2/C1 working-tree changes
> (execution-aware detection). It is recorded as its own dated baseline, not
> a measurement of `760946e`; once the B1/B2/C1 work is committed, later runs
> supersede it with clean provenance.

The August 22 B1/B2/C1 column is the first run of the execution-aware
detection program end to end:

- **Detection sizes against transit-surviving depth.** Each venue's retained
  levels are discounted by a survival rate matched to the simulator's queue
  front-running model (`venue_survival_bps`), and the raw resting depth
  travels with the signal so the fill model applies its discount exactly
  once — sizing and filling now agree by construction.
- **Multi-venue line shopping.** Every outcome draws on the best chunks
  across *all* venues and retained levels (fair coverage first, then global
  best fee-adjusted order), capped at 16 chunks; the aggregator reports the
  better of the shopped plan and the guaranteed single-best plan, so
  detector-search variance can never lose money versus one-quote-per-outcome
  selection (property-tested over 2,000 fuzzed slabs).
- **Chunk-carrying signals.** Signal events carry their full execution plan
  across the ring; every allocation is simulated against its actual leg.
- **Honest reporting.** The report shows the full disposition funnel
  (attempted → capital-short → chased → clean → partial → phantom → broken),
  dual ROI (`realized_roi_bps` and `attempted_roi_bps`), and optional
  compounding bankroll accounting (`--bankroll`), with static-budget mode
  kept for comparison.

The headline change is structural, not statistical: **clean fills went from
0 to 746 of 829** because signals are only sized against depth that survives
transit — previously every signal requested more than the fill model would
honor and settled as partial. Phantom count stays exactly at the scripted
injection rate (83 = every 10th synthetic signal), and realized PnL rises to
+$21,491.58 (+2.94%) on the same workload. Hot-loop p99 moved from ~80–100 ns
to 320 ns — the aggregator now scans all retained levels per event instead of
top-of-book only — still more than 150× inside the 50 µs budget.

The August 22 ledger column is the first **ledger-enabled** run: the pipeline now
also emits a per-trade accuracy ledger (`*.trades.jsonl`, one record per
detected-and-simulated signal), published beside the run snapshot and rendered
in the dashboard's Trades section. Detection and simulation figures are
identical to the `f9623ab` run because the synthetic stream is deterministic
and the hot path is untouched by ledger capture — the p99 difference (80 ns →
100 ns) is host scheduling noise of the same magnitude already documented in
§3. See §7 for the per-trade reconciliation.

The current run streams 2,000,000 synthetic events (the default workload
size for `cargo run --example pipeline --release`) on the reference x86_64
host (Intel i7-14700K, Linux `7.1.8-arch1-3`). A matching 200,000-tick run
was also captured for direct comparison against the earlier baselines (§3);
detection and simulator accounting are workload-size independent, since the
signal stream is deterministic and the same at both tick counts.

---

## 2. Test Environment & Configuration

### Hardware & Operating System
- **Published baseline:** Apple Silicon (macOS aarch64, M-series)
- **Linux comparisons:** x86_64 Linux (`7.1.8-arch1-3`) — Intel Core i7-14700K (20 cores / 28 threads, 5.6 GHz max boost), 46 GiB RAM
- **Memory Subsystem:** 64-byte aligned cachelines

### Compiler Flags & Profile Configuration
```toml
[profile.release]
lto = "fat"
codegen-units = 1
panic = "abort"
```

### Market Setup & Venues
- **Event:** Boston Celtics (`BOS`) vs. Los Angeles Lakers (`LAL`)
- **Market:** Moneyline 2-way proposition (`MarketId: 0`)
- **Venues & Fee Structures:**
  - **Kalshi (`VenueId: 0`):** Continuous stake fee ($350\text{ bps}$ at $50¢$), $100¢$ contract increment.
  - **Polymarket CLOB (`VenueId: 1`):** $0\text{ bps}$ maker/taker fee, $1¢$ continuous increment.
  - **Pinnacle (`VenueId: 6`):** $100\text{ bps}$ amortized stake fee, $100¢$ increment.
- **Simulator Latency Profiles:**
  - **Kalshi:** $8\text{ ms}$ wire delay, $2\text{ ms}$ venue processing, $5\%$ queue front-running degradation.
  - **Polymarket:** $12\text{ ms}$ wire delay, $3\text{ ms}$ venue processing, $10\%$ queue front-running degradation.

---

## 3. Throughput & Latency Performance

### Ingestion Throughput

| Metric | Apple Silicon baseline (200k ticks) | Linux x86_64, earlier revision (200k ticks) | Linux i7-14700K, current (200k ticks) | Linux i7-14700K, current (2M ticks) |
|---|---:|---:|---:|---:|
| Total Feed Events Ingested | 200,000 | 200,000 | 200,000 | 2,000,000 |
| Elapsed Ingestion Time | 56.60 ms | 31.51 ms | 42.81 ms | 259.02 ms |
| Burst Ingestion Throughput | 3,533,782 msg/sec | 6,347,554 msg/sec | 4,672,206 msg/sec | 7,721,309 msg/sec |

The 200k-tick current-revision figure is noisier than the 2M-tick one
(smaller sample, shared/interactive host) but both clear the earlier-revision
baseline; throughput on this workload is dominated by ring-buffer backpressure
handling, which amortizes better at larger tick counts.

### In-Process Hot Loop Latency Profile
Latency was recorded using a fixed-bin sub-microsecond histogram (`NUM_BINS = 4601`, $10\text{ ns}$ resolution) measuring the exact time from feed event ingestion to signal emission on the dedicated engine thread:

| Percentile / Metric | Apple Silicon baseline (200k ticks) | Linux x86_64, earlier revision (200k ticks) | Linux i7-14700K, current (200k ticks) | Linux i7-14700K, current (2M ticks) | Target Budget | Result |
|---|---:|---:|---:|---:|---:|---|
| **Min Latency** | `0.166 µs` (166 ns) | `0.092 µs` (92 ns) | `0.013 µs` (13 ns) | `0.013 µs` (13 ns) | — | — |
| **p50 (Median)** | `0.200 µs` (200 ns) | `0.090 µs` (90 ns) | `0.050 µs` (50 ns) | `0.050 µs` (50 ns) | — | **Sub-microsecond** |
| **p90** | `0.250 µs` (250 ns) | `0.100 µs` (100 ns) | `0.060 µs` (60 ns) | `0.060 µs` (60 ns) | — | **Sub-microsecond** |
| **p99** | **`0.250 µs` (250 ns)** | **`0.100 µs` (100 ns)** | **`0.070 µs` (70 ns)** | **`0.080 µs` (80 ns)** | **`< 50.000 µs`** | **PASSED on all hosts** |
| **p99.9** | `0.500 µs` (500 ns) | `0.480 µs` (480 ns) | `1.270 µs` (1,270 ns) | `0.120 µs` (120 ns) | — | **Sub-microsecond** |
| **Max Latency** | `0.500 µs` (500 ns) | `0.486 µs` (486 ns) | `5.468 µs` (5,468 ns) | `2639.983 µs` (2.64 ms, outlier) | — | **See note** |
| **Mean Latency** | `0.216 µs` (216 ns) | `0.097 µs` (97 ns) | `0.060 µs` (60 ns) | `0.057 µs` (57 ns) | — | **Deterministic** |

The 2M-tick run's max latency is a single-event scheduling-jitter outlier
(this host is shared/interactive, not isolated) — p99.9 stays at 120 ns, so
it does not reflect a systemic tail. Re-running the 2M-tick workload twice
more produced max outliers of 20.6 µs and 32.2 µs with p99 stable at
90–100 ns, confirming the outlier is host scheduling noise, not a hot-loop
regression. p99 remains **more than 600x inside** the 50 µs budget on every
run recorded here.

---

## 4. Arbitrage Detection Metrics

The detector evaluated each tick against resting book depth, venue fees, and contract sizing increments.

Apple Silicon / earlier-revision Linux baselines (200k ticks):

```
Total Feed Events Processed:        200,004
Valid Signals Emitted:                 829
Collected Signal Events:               829
Sample Signal Raw Edge:                 440 bps (Kalshi 46¢ + Polymarket 48¢ = 94¢ raw)
Sample Signal Fee Cut:                 -164 bps (Kalshi 350 bps stake fee adjustment)
Sample Signal Net Edge:                  22 bps (Worst-case guaranteed net return)
Sample Total Stake:                  99,940 cents ($999.40)
Sample Guaranteed Worst-Case PnL:       227 cents ($2.27)
```

Current run, commit `f9623ab` (both 200k- and 2M-tick streams — identical
detection outcome, since the signal stream is workload-size independent):

```
Total Feed Events Processed:  200,004 (200k run) / 2,000,004 (2M run)
Valid Signals Emitted:                 829
Collected Signal Events:               829
Sample Signal Net Edge:                  28 bps (Worst-case guaranteed net return)
Sample Total Stake:                  98,310 cents ($983.10)
Sample Guaranteed Worst-Case PnL:       280 cents ($2.80)
```

Signal counts (829) match the earlier baselines exactly — the arbitrage
windows in the synthetic stream are unchanged. The sample edge and PnL
figures differ (22 bps → 28 bps net, $2.27 → $2.80) because the detector/fee
math changed between the earlier-revision snapshot and commit `f9623ab`; this
is a real change in reported figures, not a measurement artifact, and is
consistent across both tick counts recorded today.

---

## 5. Paper-Trading Simulator & Execution Accounting

Simulated executions accounted for transit time, queue position front-running, and book decay:

### Fill & Phantom Breakdown
```
Total Signals Simulated:               829
Fully Clean Fills:                     746   (B1/B2/C1 run; was 0 in every prior baseline)
Proportional / Partial Fills:            0   (was 746 — sizing no longer overdraws depth)
Phantom Signals (Decayed in Flight):    83
Measured Phantom Rate:               10.01% (1,001 bps)
```

Fill and phantom counts are unchanged from every prior baseline recorded in
this document **except** under the B1/B2/C1 execution-aware detector: with
sizing matched to transit-surviving depth, the 746 trades that previously
settled as partial fills now fill clean, and phantoms remain exactly the
scripted synthetic injection (every 10th signal).

### Cumulative Financial Ledger
All balances computed using pure integer `Cents` (`i64`):

| Metric | Earlier baselines (all hosts) | Ledger run — `0b0306e` | B1/B2/C1 run (2M ticks) |
|---|---:|---:|---:|
| Cumulative Staked | 72,876,755 cents ($728,767.55) | 72,856,290 cents ($728,562.90) | 72,919,523 cents ($729,195.23) |
| Total Venue Fees Paid | 2,605,032 cents ($26,050.32) | 2,605,032 cents ($26,050.32) | 2,627,842 cents ($26,278.42) |
| Realized Worst-Case Profit | 1,550,173 cents (+$15,501.73) | 1,570,638 cents (+$15,706.38) | **2,149,158 cents (+$21,491.58)** |
| Realized Settlement ROI | +2.12% | +2.15% | **+2.94%** |

---

## 6. Full Workspace Test Verification Matrix

All 170 tests across the five workspace crates passed with zero errors and zero linter warnings (`cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`), up from 159 in the previous baseline as workspace test coverage has grown:

```
running 47 tests in arbkit-core (unittests) ................. passed
running 11 tests in arbkit-core (properties) ................ passed
running  1 test  in arbkit-core (doctests) .................. passed
running 21 tests in arbkit-engine (unittests) ............... passed
running  6 tests in arbkit-engine (engine_tests) ............ passed
running 13 tests in arbkit-feed (unittests) .................. passed
running  4 tests in arbkit-feed (feed_tests) ................. passed
running  2 tests in arbkit-feed (properties) ................. passed
running 18 tests in arbkit-match (unittests) ................. passed
running  6 tests in arbkit-match (integration) ............... passed
running  4 tests in arbkit-match (properties) ................ passed
running 23 tests in arbkit-sim (unittests) .................... passed
running 14 tests in arbkit-sim (sim_tests) .................... passed
----------------------------------------------------------------------------
Total: 170 passed, 0 failed, 0 ignored, 0 clippy warnings
```

---

## 7. Per-Trade Accuracy Ledger (August 22, 2026)

The `0b0306e` run is the first recorded with ledger capture enabled: the
pipeline writes one JSONL record per detected-and-simulated signal, pairing
each engine signal event with its paper-trading execution report. The ledger
is a static asset (`2026-08-22T005623-945Z-linux-x86_64-0b0306e.trades.jsonl`)
published beside the run snapshot and rendered trade-by-trade in the
dashboard's Trades section.

Ledger contents and reconciliation against the aggregate simulation section:

```
Trades written:                        829 (header count matches line count)
Profitable trades:                     746
Classification:            746 proportional / 83 phantom / 0 clean / 0 brokenLeg
Ledger realized PnL total:       1,570,638 cents (+$15,706.38)
Simulation realized PnL total:   1,570,638 cents  -> reconciles exactly
```

Every money field in the ledger is the pipeline's own integer cents; every
rate field its integer bps/ppm. Nothing is rounded, recomputed, or
float-formatted between the engine, the JSONL file, and the dashboard.
Pre-ledger runs (all snapshots before this one) show an honest "no trade log
recorded for this run" state rather than synthesized rows.
