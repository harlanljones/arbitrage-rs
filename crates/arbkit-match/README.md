# arbkit-match

[![Live Demo](https://img.shields.io/badge/Live%20Demo-arbkit.harlanljones.com-0ea5e9?style=flat&logo=cloudflare)](https://arbkit.harlanljones.com/)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](../../README.md#license)

Canonical event registry, team name alias normalization, and string-to-ID interning for [`arbkit`](../../README.md).

> 🌐 **Live Demo:** [https://arbkit.harlanljones.com/](https://arbkit.harlanljones.com/)

---

## Features

- **Team Alias Dictionary:** Normalizes team names across formats (e.g. `LAL @ BOS`, `Boston Celtics vs Los Angeles Lakers`, `KXNBAGAME-26AUG18BOSLAL`).
- **Canonical Market Registry:** Links disparate exchange markets to a shared `MarketId` and `OutcomeId`.
- **Spread & Totals Line Mirroring:** Validates that point spreads and totals match perspectives (e.g. Celtics -3.5 vs Lakers +3.5).
- **String Interner & Hot Lookup:** Converts dynamic strings to `u32`/`u16` IDs ahead of time, ensuring $O(1)$ zero-allocation lookups in the engine hot loop.
