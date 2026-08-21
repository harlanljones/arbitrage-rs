# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Stack

The core project is a Rust 2021 workspace. Its public results dashboard uses React, TypeScript, and Vite, ships as static assets on Cloudflare Workers, and deploys through Workers Builds.

## Users

The dashboard serves two audiences equally: public evaluators deciding whether the project is credible and engineers inspecting its methodology, performance, and reproducibility.

## Product Purpose

`arbkit` detects cross-venue sports and prediction-market arbitrage after accounting for fees, available depth, contract granularity, venue matching, and stale data. Success means producing pessimistic, reproducible paper-trading evidence while keeping the in-process hot path comfortably below its 50 microsecond p99 budget.

## Positioning

The project separates canonical market matching from a fixed-point, zero-allocation detection hot path, then subjects emitted signals to latency and queue-decay simulation instead of presenting raw theoretical edges as trades.

## Operating Context

Engineers run the release pipeline locally, review the console report and generated JSON snapshot, then commit approved dated results. Cloudflare Workers Builds publishes the static dashboard from repository history. Live order placement is not part of the project.

## Capabilities and Constraints

- Five Rust crates cover pricing and detection, matching, feeds, the engine, and paper-trading simulation.
- Benchmark runs are host-specific and must carry environment and workload provenance.
- Dashboard results are generated, reviewed, and committed; there is no live upload API or Cloudflare data store.
- Synthetic workloads and paper-trading results must always be labeled as such.
- Different-host measurements may be compared, but must not be presented as same-machine performance trends.

## Brand Commitments

The name is `arbkit`. The voice is technically precise, skeptical of theoretical-only arbitrage claims, and explicit about pessimistic accounting, measurement boundaries, and limitations.

## Evidence on Hand

- `RESULTS.md` contains dated Apple Silicon and x86_64 Linux benchmark results, simulator accounting, and the 114-test verification matrix.
- `README.md` contains the project status, latency budget, performance highlights, and scope limitations.
- `ARCHITECTURE.md` documents the workspace, hot-path invariants, and end-to-end data flow.
- `crates/arbkit-engine/examples/pipeline.rs` is the executable source of generated benchmark and paper-trading metrics.
- No live-trading performance, customer claims, testimonials, or production order-placement evidence exists and none may be fabricated.

## Product Principles

- Prove performance with dated measurements and visible provenance.
- Distinguish theoretical signals from executable paper-trading outcomes.
- Prefer pessimistic, integer-based accounting over optimistic estimates.
- Let public readers understand the result quickly while keeping the methodology inspectable.
- Preserve the Rust hot path's dependency and allocation discipline.

## Accessibility & Inclusion

The dashboard must support keyboard navigation, reduced motion, responsive reading, sufficient contrast, and text/table equivalents for graphical evidence.
