# `arbkit`: Architecture, Performance, & Simulation Report

**Version:** 0.1.0  
**Rust Edition:** 2021 (Rustc 1.97.1)  
**Profile:** Release (Fat LTO, `codegen-units = 1`, `panic = "abort"`)  
**Repository:** `https://github.com/harlanljones/arbkit`  
**Live Demo & Results Ledger:** `https://arbkit.harlanljones.com/`

---

## Executive Summary

`arbkit` is an ultra-low latency, deterministic cross-venue sports and prediction market arbitrage detection and paper-trading engine written in pure Rust.

The system ingests streaming order book snapshots and delta updates from exchanges (e.g., Kalshi, Polymarket CLOB) and aggregators, normalizes disparate quote formats and proposition representations, detects cross-venue mispricings in sub-microsecond time, and evaluates fill feasibility through an execution simulator accounting for queue front-running and asymmetric transit delays. Interactive benchmark distributions and proof ledgers can be explored live at [arbkit.harlanljones.com](https://arbkit.harlanljones.com/).

```mermaid
flowchart LR
    subgraph Boundary["Feed Boundary (I/O & Ingress)"]
        K[Kalshi Feed] --> KP[Kalshi Parser]
        P[Polymarket Feed] --> PP[Polymarket Parser]
        T[Binary Tape Player] --> TP[Tape Codec]
    end

    subgraph HotPath["Engine Hot Loop (Thread Pinned, Zero Allocation)"]
        KP & PP & TP -->|Try Push| IngressRing["Lock-Free SPSC Ring<br/>(Cacheline-Padded)"]
        IngressRing -->|Try Pop| Engine["Hot Engine Core"]
        Engine <-->|O(1) Direct Index| Slab["Preallocated Slab<br/>(Flat Book Storage)"]
        Engine -->|Detect| Detector["Pessimistic Arb Detector<br/>(Integer Arithmetic)"]
        Detector -->|Signal Event| EgressRing["Lock-Free SPSC Ring<br/>(Signal Queue)"]
    end

    subgraph Downstream["Simulator & Analytics"]
        EgressRing --> Sim["Paper Trading Simulator<br/>(Latency & Queue Model)"]
        Sim --> Analytics["PnL & Phantom Ledger<br/>(Pessimistic Cents Accounting)"]
    end
```

---

## 1. Workspace Architecture & Crate Responsibilities

The codebase is organized into five specialized crates enforcing strict separation of concerns and dependency isolation:

```
crates/arbkit-core     Prices, books, fees, detection. No I/O, no clock, no network.
crates/arbkit-match    Canonical event registry, team normalizer, string-to-ID interning.
crates/arbkit-feed     Polymarket and Kalshi parsers, binary tape recorder and player.
crates/arbkit-engine   Lock-free SPSC ring buffers, preallocated book slab, hot loop, latency histogram.
crates/arbkit-sim      Paper trading simulator, latency modeling, phantom-rate measurement.
```

### Dependency Isolation Guarantees

| Crate | Primary Focus | Hot-Path Invariants | External Dependencies |
|---|---|---|---|
| [`arbkit-core`](crates/arbkit-core) | Domain model, fixed-point prices, fee math, arb detector | **No I/O, no network, no clock, no allocations.** | `thiserror` only |
| [`arbkit-match`](crates/arbkit-match) | Canonical event/market registry, team aliases, line mirroring | Interns strings to `u32`/`u16` IDs before engine loop. | `arbkit-core`, `thiserror` |
| [`arbkit-feed`](crates/arbkit-feed) | Wire message parsing & zero-allocation binary tape | Stack-allocated `Copy` events (`FeedEvent`). | `arbkit-core`, `arbkit-match`, `serde`, `serde_json` |
| [`arbkit-engine`](crates/arbkit-engine) | Hot loop, SPSC ring, flat slab, latency histogram | **No locks, no allocations, single-threaded execution.** | `arbkit-core`, `arbkit-match`, `arbkit-feed`, `thiserror` |
| [`arbkit-sim`](crates/arbkit-sim) | Paper trading, queue front-running, phantom rate | Pure integer `Cents` accounting, zero floats in logic. | `arbkit-core`, `arbkit-match`, `arbkit-feed`, `arbkit-engine` |

> [!IMPORTANT]
> CI validates via `cargo tree` that `arbkit-core` never picks up an I/O crate (`tokio`, `reqwest`, `hyper`, `mio`, `socket2`, `tungstenite`). The core domain runs offline in milliseconds and is verified across 100% deterministic test suites.

---

## 2. Core Mathematical & Domain Innovations

### A. Fixed-Point Integer Pricing (`Prob` & `Odds`)
Textbook arbitrage formulas evaluate whether $\sum \frac{1}{\text{odds}_i} < 1.0$. Performing this in standard `f64` floating-point arithmetic is vulnerable to non-associativity and precision drift that manufactures non-existent edges.

- `Prob`: Implied probability in **parts per million (ppm)** stored as `u32` ($\text{ppm} \in [1, 1\,000\,000]$).
- `Odds`: Micro-unit decimal odds stored as `u64` ($\text{micro} \in [1\,000\,000, 1\,000\,000\,000\,000]$).
- Exact integer reciprocal relationship: $\text{ppm} \times \text{micro} \approx 10^{12}$ with zero-bias rounded division (`div_round`).

### B. Pre-Comparison Venue Fee Adjustments
Raw-quote detectors generate streams dominated by false edges. Fees must be applied to each leg *before* the overround summation:

1. **Exchange Net Winnings Commission** (Betfair model): $\text{Odds}_{\text{eff}} = 1 + (d - 1)(1 - c)$
2. **Stake Fee Model**: $\text{Odds}_{\text{eff}} = \frac{d}{1 + f}$
3. **Kalshi Per-Contract Model**: $\text{Fee} = \lceil 0.07 \times C \times P \times (1 - P) \rceil$, which reduces to $700 \times (1 - P)\text{ bps}$ of stake. This peaks at $350\text{ bps}$ for $50¢$ contracts and penalizes low-probability contracts even more heavily.

### C. Pessimistic Sizing & Payoff Equalization
- **Depth Capping**: Sized against the thinnest leg's resting liquidity.
- **Granularity Truncation**: Each leg's stake is rounded down to integer contract increments (`increment`).
- **Guaranteed Profit Floor**: Reported profit is the **worst-case payout across any outcome** minus the total stake:
$$\text{Worst-Case Profit} = \min_{i} (\text{Payout}_i) - \sum_{j} \text{Stake}_j$$

### D. Staleness as a First-Class Safety State
When a feed skips a sequence number, `OutcomeBook` immediately transitions to `stale = true` and suppresses quote emissions. Gaps degrade into silence rather than confidently incorrect trades.

---

## 3. Hot-Path Low-Latency Engineering

The hot loop (`crates/arbkit-engine`) is engineered to satisfy a **$p99 < 50\text{ }\mu\text{s}$** latency budget:

1. **Zero Heap Allocation**:
   - Fixed-size arrays throughout (`[Level; 8]`, `[Leg; 4]`, `[Allocation; 4]`).
   - `EngineSlab` preallocates all books in a flat vector indexed via arithmetic:
     $$\text{Index} = (\text{market\_id} \times \text{MAX\_OUTCOMES} + \text{outcome\_id}) \times \text{MAX\_VENUES} + \text{venue\_id}$$
2. **Lock-Free SPSC Queues**:
   - Zero-allocation turn-sequenced atomic ring buffers with acquire-release memory ordering.
   - Slots and pointers are aligned to 64 bytes (`#[repr(align(64))]`) to eliminate CPU cacheline bouncing and false sharing.
3. **Synchronous Pinned Loop**:
   - The hot engine loop executes on a dedicated OS thread without async runtimes (`tokio` terminates at the feed edge).

---

## 4. Benchmark & Performance Results

### Benchmark Environment
- **Published baseline:** Apple Silicon (macOS aarch64, M-series), August 19, 2026
- **Current comparison:** x86_64 Linux (`7.1.8-arch1-3`), August 21, 2026
- **Profile:** `--release` with Fat LTO and single codegen unit
- **Dataset:** 200,000 sequenced market events across Kalshi and Polymarket order books

| Metric | Apple Silicon baseline | Current Linux run | Budget / Expected |
|---|---:|---:|---|
| Elapsed ingestion time | 56.60 ms | 31.51 ms | — |
| Burst ingestion throughput | 3,533,782 msg/sec | 6,347,554 msg/sec | High-frequency burst ingestion |
| Hot-loop p50 | 0.200 µs | 0.090 µs | Sub-microsecond |
| Hot-loop p99 | 0.250 µs | 0.100 µs | < 50.000 µs (PASS) |
| Hot-loop max | 0.500 µs | 0.486 µs | No long tails |
| Valid signals emitted | 829 | 829 | Deterministic |
| Phantom rate | 10.01% | 10.01% | Deterministic simulation |
| Realized worst-case PnL | +$15,501.73 | +$15,501.73 | Net of fees and rounding |
| Worst-case settlement ROI | +2.12% | +2.12% | Deterministic simulation |

---

## 5. Verification & Test Suite Summary
 
The entire workspace is verified through unit tests, integration pipelines, doc tests, and property tests:
 
| Test Suite | Scope | Tests Run | Result |
|---|---|---|---|
| **`arbkit-core`** | Price conversions, fee models, book sequence gaps, arb detection, doctests | 59 tests | **Passed** |
| **`arbkit-match`** | Team alias dictionary, line mirroring, cross-venue market registry | 28 tests | **Passed** |
| **`arbkit-feed`** | Kalshi/Polymarket parsers, binary tape codec roundtrips | 19 tests | **Passed** |
| **`arbkit-engine`** | SPSC ring cross-thread concurrency, slab indexing, hot loop | 16 tests | **Passed** |
| **`arbkit-sim`** | Wire latency modeling, queue depth depletion, PnL accounting | 37 tests | **Passed** |
| **Workspace Total** | **All targets, doctests, and integration suites** | **159 / 159** | **100% Passed (0 warnings)** |

---

## 6. Milestone Progress

- [x] **M0** — Workspace configuration, CI, toolchain.
- [x] **M1** — `arbkit-core`: prices, fees, books, detection, property tests.
- [x] **M2** — `arbkit-feed`: Polymarket and Kalshi parsers, binary tape recorder/player.
- [x] **M3** — `arbkit-match`: canonical event registry and string-to-ID interning.
- [x] **M4** — `arbkit-engine`: the hot loop, lock-free SPSC rings, latency histogram.
- [x] **M5** — `arbkit-sim`: paper trading, queue degradation, phantom-rate measurement.
- [x] **M6** — Benchmarking & tuning against the $50\text{ }\mu\text{s}$ budget (Achieved: **$p99 = 0.10–0.25\text{ }\mu\text{s}$** across measured hosts).
