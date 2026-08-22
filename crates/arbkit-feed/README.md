# arbkit-feed

[![Live Demo](https://img.shields.io/badge/Live%20Demo-arbkit.harlanljones.com-0ea5e9?style=flat&logo=cloudflare)](https://arbkit.harlanljones.com/)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](../../README.md#license)

Streaming WebSocket wire parsers and zero-allocation binary tape recorder/player for [`arbkit`](../../README.md).

> 🌐 **Live Demo:** [https://arbkit.harlanljones.com/](https://arbkit.harlanljones.com/)

---

## Features

- **Venue Connectors & Parsers:**
  - **Kalshi:** Sequence-tracked JSON deltas, snapshots, and heartbeats with sequence-gap detection.
  - **Polymarket CLOB:** Level-2 market channel book updates and trade messages.
- **Copy Event Boundary:** Converts wire payloads into stack-allocated `FeedEvent` instances with no dynamic heap allocations.
- **Binary Tape Codec:** Deterministic record and replay format with CRC32 checksum verification for regression testing and offline simulation.
