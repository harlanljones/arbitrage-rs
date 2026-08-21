# `arbkit`: Benchmark & Pipeline Execution Results

**Version:** 0.1.0  

**Test Dates:** August 19, 2026 (baseline); August 21, 2026 (current run); August 21, 2026 (optimized 2M-tick run)

**Toolchain:** `rustc 1.97.1` (`aarch64-apple-darwin` baseline; `x86_64-unknown-linux-gnu` current)

**Build Profile:** `release` (`lto = "fat"`, `codegen-units = 1`, `panic = "abort"`)  

---

## 1. Executive Summary

This report documents the live end-to-end pipeline execution results of `arbkit` across market data ingestion, single-threaded hot loop detection, sub-microsecond latency measurement, and execution simulation.

| Metric | Apple Silicon baseline (200k ticks) | Linux x86_64 run (200k ticks) | Linux i7-14700K optimized (2M ticks) |
|---|---:|---:|---:|
| Ingestion Throughput | 3,533,782 msg/sec | 6,347,554 msg/sec | 12,371,271 msg/sec |
| Hot Loop Latency (p99) | 0.250 µs (250 ns) | 0.100 µs (100 ns) | 0.100 µs (100 ns) |
| Hot Loop Latency (Median / p50) | 0.200 µs (200 ns) | 0.090 µs (90 ns) | 0.090 µs (90 ns) |
| Measured Phantom Rate | 10.01% (1,001 bps) | 10.01% (1,001 bps) | 10.01% (1,001 bps) |
| Paper-Trading Realized PnL | +$15,501.73 | +$15,501.73 | +$15,501.73 |
| Realized Settlement ROI | +2.12% | +2.12% | +2.12% |
| Workspace Test Verification | 114 / 114 passed | 114 / 114 passed | 114 / 114 passed |

The optimized run raises the default synthetic stream from 200,000 to
2,000,000 events — the point at which burst-ingestion throughput plateaus and
latency percentiles stabilize on the reference x86_64 host (Intel i7-14700K,
Linux `7.1.8-arch1-3`). Simulator accounting is unchanged because the signal
stream is deterministic and workload-size independent.

---

## 2. Test Environment & Configuration

### Hardware & Operating System
- **Published baseline:** Apple Silicon (macOS aarch64, M-series)
- **Current comparison:** x86_64 Linux (`7.1.8-arch1-3`)
- **Optimized 2M-tick run:** same x86_64 Linux host — Intel Core i7-14700K (20 cores / 28 threads, 5.6 GHz max boost), 46 GiB RAM
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

| Metric | Apple Silicon baseline (200k ticks) | Linux x86_64 run (200k ticks) | Linux i7-14700K optimized (2M ticks) |
|---|---:|---:|---:|
| Total Feed Events Ingested | 200,000 | 200,000 | 2,000,000 |
| Elapsed Ingestion Time | 56.60 ms | 31.51 ms | 161.66 ms |
| Burst Ingestion Throughput | 3,533,782 msg/sec | 6,347,554 msg/sec | 12,371,271 msg/sec |

### In-Process Hot Loop Latency Profile
Latency was recorded using a fixed-bin sub-microsecond histogram (`NUM_BINS = 4601`, $10\text{ ns}$ resolution) measuring the exact time from feed event ingestion to signal emission on the dedicated engine thread:

| Percentile / Metric | Apple Silicon baseline (200k ticks) | Linux x86_64 run (200k ticks) | Linux i7-14700K optimized (2M ticks) | Target Budget | Result |
|---|---:|---:|---:|---:|---|
| **Min Latency** | `0.166 µs` (166 ns) | `0.092 µs` (92 ns) | `0.092 µs` (92 ns) | — | — |
| **p50 (Median)** | `0.200 µs` (200 ns) | `0.090 µs` (90 ns) | `0.090 µs` (90 ns) | — | **Sub-microsecond** |
| **p90** | `0.250 µs` (250 ns) | `0.100 µs` (100 ns) | `0.100 µs` (100 ns) | — | **Sub-microsecond** |
| **p99** | **`0.250 µs` (250 ns)** | **`0.100 µs` (100 ns)** | **`0.100 µs` (100 ns)** | **`< 50.000 µs`** | **PASSED on all hosts** |
| **p99.9** | `0.500 µs` (500 ns) | `0.480 µs` (480 ns) | `0.740 µs` (740 ns) | — | **Sub-microsecond** |
| **Max Latency** | `0.500 µs` (500 ns) | `0.486 µs` (486 ns) | `0.744 µs` (744 ns) | — | **No long tails** |
| **Mean Latency** | `0.216 µs` (216 ns) | `0.097 µs` (97 ns) | `0.097 µs` (97 ns) | — | **Deterministic** |

---

## 4. Arbitrage Detection Metrics

The detector evaluated each tick against resting book depth, venue fees, and contract sizing increments.

200k-tick runs (both baselines):

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

Optimized 2M-tick run (Linux i7-14700K):

```
Total Feed Events Processed:      2,000,004
Valid Signals Emitted:                 829
Collected Signal Events:               829
Sample Signal Net Edge:                  22 bps (Worst-case guaranteed net return)
Sample Total Stake:                  99,940 cents ($999.40)
Sample Guaranteed Worst-Case PnL:       227 cents ($2.27)
```

Signal counts are workload-size independent: the detector emits on arbitrage-window state transitions, so the larger stream exercises the same deterministic signal sequence.

---

## 5. Paper-Trading Simulator & Execution Accounting

Simulated executions accounted for transit time, queue position front-running, and book decay:

### Fill & Phantom Breakdown
```
Total Signals Simulated:               829
Fully Clean Fills:                       0
Proportional / Partial Fills:          746 (Hedges preserved across remaining depth)
Phantom Signals (Decayed in Flight):    83
Measured Phantom Rate:               10.01% (1,001 bps)
```

### Cumulative Financial Ledger
All balances computed using pure integer `Cents` (`i64`):

```
Cumulative Staked:            72,876,755 cents ($728,767.55)
Total Venue Fees Paid:         2,605,032 cents ($26,050.32)
Realized Worst-Case Profit:    1,550,173 cents (+$15,501.73)
Realized Settlement ROI:          +2.12%
```

---

## 6. Full Workspace Test Verification Matrix

All 114 tests across the five workspace crates passed with zero errors and zero linter warnings:

```
running 28 tests in arbkit-core (unittests) ................. passed in 0.00s
running  8 tests in arbkit-core (properties) ................ passed in 0.03s
running  1 test  in arbkit-core (doctests) .................. passed in 0.37s
running 10 tests in arbkit-engine (unittests) ............... passed in 0.01s
running  4 tests in arbkit-engine (engine_tests) ............ passed in 0.01s
running 13 tests in arbkit-feed (unittests) ................. passed in 0.00s
running  4 tests in arbkit-feed (feed_tests) ................ passed in 0.00s
running  2 tests in arbkit-feed (properties) ................ passed in 0.04s
running 18 tests in arbkit-match (unittests) ................ passed in 0.00s
running  6 tests in arbkit-match (integration) .............. passed in 0.00s
running  4 tests in arbkit-match (properties) ............... passed in 0.04s
running  6 tests in arbkit-sim (unittests) .................. passed in 0.00s
running 10 tests in arbkit-sim (sim_tests) .................. passed in 0.00s
----------------------------------------------------------------------------
Total: 114 passed, 0 failed, 0 ignored, 0 clippy warnings
```
