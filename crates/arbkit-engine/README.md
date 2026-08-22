# arbkit-engine

[![Live Demo](https://img.shields.io/badge/Live%20Demo-arbkit.harlanljones.com-0ea5e9?style=flat&logo=cloudflare)](https://arbkit.harlanljones.com/)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](../../README.md#license)

Sub-microsecond hot loop engine, cacheline-aligned lock-free SPSC rings, preallocated flat book slab, and fixed-bin latency histogram for [`arbkit`](../../README.md).

> 🌐 **Live Demo:** [https://arbkit.harlanljones.com/](https://arbkit.harlanljones.com/)

---

## Low-Latency Architecture

- **Dedicated OS Thread:** The engine hot loop runs synchronously on a pinned core without async runtime interference.
- **Lock-Free SPSC Queues:** Cacheline-aligned (`#[repr(align(64))]`) ring buffers with acquire-release atomics preventing false sharing.
- **Flat Memory Slab:** `EngineSlab` preallocates contiguous order book storage indexed in $O(1)$ via flat arithmetic:
  $$\text{Index} = (\text{market\_id} \times \text{MAX\_OUTCOMES} + \text{outcome\_id}) \times \text{MAX\_VENUES} + \text{venue\_id}$$
- **High-Resolution Latency Histogram:** Sub-microsecond fixed-bin histogram tracking p50, p90, p99, p99.9, and max service times down to 10 ns buckets.
- **Measured Latency:** P99 of **70–100 ns** on Linux x86_64 and **250 ns** on Apple Silicon (>200× to >600× inside the 50 µs budget).
