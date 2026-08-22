# arbkit-sim

[![Live Demo](https://img.shields.io/badge/Live%20Demo-arbkit.harlanljones.com-0ea5e9?style=flat&logo=cloudflare)](https://arbkit.harlanljones.com/)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](../../README.md#license)

Execution simulator, asymmetric wire latency modeling, queue front-running decay, and paper-trading accounting for [`arbkit`](../../README.md).

> 🌐 **Live Demo:** [https://arbkit.harlanljones.com/](https://arbkit.harlanljones.com/)

---

## Features

- **Realistic Latency Modeling:** Models discrete network wire delays, matching engine processing time, and asymmetric round-trip arrivals per venue.
- **Queue Front-Running & Book Degradation:** Accounts for depth loss while orders travel in-flight, converting unfillable opportunities to phantom signals.
- **Strict Integer PnL Ledger:** Pure integer `Cents` (`i64`) accounting ensuring zero floating-point accumulation errors across stakes, fee deductions, and payouts.
- **Hedging Preservation:** Proportionally scales multi-leg fills to preserve guaranteed equal payoffs across outcomes.
