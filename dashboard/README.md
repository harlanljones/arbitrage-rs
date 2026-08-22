# arbkit-dashboard

[![Live Demo](https://img.shields.io/badge/Live%20Demo-arbkit.harlanljones.com-0ea5e9?style=flat&logo=cloudflare)](https://arbkit.harlanljones.com/)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](../README.md#license)

The interactive proof ledger and benchmark results visualizer for the [`arbkit`](../README.md) Rust workspace.

> 🌐 **Production Deployment:** [https://arbkit.harlanljones.com/](https://arbkit.harlanljones.com/)

---

## Features

- **Audited Proof Ledger:** Visualizes the p99 latency against the 50 µs budget ruler with exact headroom calculation.
- **Latency Distribution Plot:** Tail percentiles (p50, p90, p99, p99.9, p99.99, max) across dated benchmark snapshots.
- **Host-to-Host Throughput:** High-frequency burst ingestion comparison across architectures (Apple Silicon and Linux x86_64).
- **Execution & Phantom Breakdown:** Proportional fills vs. queue-decayed phantom signals.
- **Pessimistic Financial Accounting:** Realized worst-case settlement PnL computed strictly in integer cents.
- **Workspace Verification Matrix:** Live pass/fail and clippy status across all workspace crates.

---

## Development

```bash
# Install dependencies
npm install

# Run local development server
npm run dev

# Run unit and component tests
npm test

# Run end-to-end tests
npm run test:e2e

# Build production bundle
npm run build
```

---

## Recording a Benchmark Snapshot

To execute the Rust release pipeline on the local machine, validate the resulting JSON report schema, and append a versioned snapshot to `public/data/runs/`:

```bash
npm run record
```

Review the updated `public/data/runs/index.json` and generated snapshot file before committing.

---

## Cloudflare Workers Deployment

The dashboard builds as a static Single Page Application (SPA) served via Cloudflare Workers:

```bash
# Dry run deploy
npm run deploy:dry

# Deploy to Cloudflare Workers
npm run deploy
```

Configuration is defined in [`wrangler.jsonc`](wrangler.jsonc).
