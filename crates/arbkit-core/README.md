# arbkit-core

[![Live Demo](https://img.shields.io/badge/Live%20Demo-arbkit.harlanljones.com-0ea5e9?style=flat&logo=cloudflare)](https://arbkit.harlanljones.com/)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](../../README.md#license)

Domain core for [`arbkit`](../../README.md): odds arithmetic, fixed-point probability representations, venue fee models, and pessimistic arbitrage detection.

> 🌐 **Live Demo:** [https://arbkit.harlanljones.com/](https://arbkit.harlanljones.com/)

---

## Invariants & Design

- **Zero I/O, No Clocks, No Allocations:** The hot path runs completely in memory with zero allocations and no networking dependencies (`thiserror` only).
- **Fixed-Point Arithmetic:** `Prob` (implied probability in ppm, $1\text{ to }1\,000\,000$) and `Odds` (micro-unit decimal odds, $1\,000\,000\text{ to }10^{12}$) eliminate floating-point drift.
- **Pessimistic Calculations:**
  - Stake is capped by the thinnest book leg.
  - Granularity truncation rounds legs down to tradeable contract increments.
  - Reported profit is the *worst-case outcome payout* minus total staked capital.
- **Pre-Comparison Fee Models:**
  - Kalshi per-contract model: $\lceil 0.07 \times C \times P \times (1-P) \rceil$.
  - Exchange winnings commission (Betfair model).
  - Continuous stake fees.

---

## Usage Example

```rust
use arbkit_core::{detect, Fee, Leg, Prob};

let legs = [
    Leg {
        venue: 0,
        outcome: 0,
        quoted: Prob::from_cents(48)?,
        fee: Fee::StakeFeeBps(364),
        capacity: 120_000,
        increment: 48,
    },
    Leg {
        venue: 1,
        outcome: 1,
        quoted: Prob::from_cents(50)?,
        fee: Fee::CommissionBps(200),
        capacity: 500_000,
        increment: 1,
    },
];

match detect(&legs, 100_000)? {
    Some(signal) => println!("{} bp net edge on ${}", signal.profit_bps, signal.total_stake / 100),
    None => println!("No viable edge after fees and rounding"),
}
# Ok::<(), arbkit_core::CoreError>(())
```
