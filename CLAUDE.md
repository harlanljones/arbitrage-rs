# arbkit

Cross-venue sports arbitrage detection. See `README.md` for what it does and
`crates/arbkit-core` for the domain model.

## Layout

- `crates/arbkit-core` — prices, books, fees, arbitrage detection. **No I/O, ever.**
- `crates/arbkit-match` — canonical event registry, team alias normalizer, string interning.
- `crates/arbkit-feed` — venue connectors (Kalshi, Polymarket CLOB), binary tape recorder/player.
- `crates/arbkit-engine` — lock-free SPSC ring, flat slab, hot loop, sub-microsecond latency histogram.
- `crates/arbkit-sim` — paper trading simulator, queue front-running, phantom rate analytics.

## Hot path rules

The engine loop runs on one pinned thread and is budgeted at **p99 < 50 µs**
from socket read to signal emitted. These rules are what defend that number.
Code on the path from a feed message to a `Signal` must not:

1. **Allocate.** Everything is preallocated at startup — fixed arrays, slabs
   indexed by interned id. No `Vec` growth, no `Box`, no `String`.
2. **Lock.** Feeds hand off over lock-free SPSC ring buffers. No `Mutex`, no
   `RwLock`, no `Arc` traffic.
3. **Be async.** `tokio` stops at the feed boundary. The engine loop is a plain
   thread with a plain loop.
4. **Hold strings.** Venue symbols are interned to `u32` at the boundary by
   `arbkit-match`. The loop never sees, hashes, or compares a `&str`.
5. **Log inline.** Push a small `Copy` record onto a ring; a writer thread
   formats it.
6. **Use floating point in a decision.** `f64` is allowed at the feed boundary
   (JSON gives us no choice) and in `as_f64` display accessors. It is not
   allowed in anything that decides whether to trade — see the `price` module
   docs for why.

Release builds use `lto = "fat"`, `codegen-units = 1`, `panic = "abort"`. A
panic on the hot path is the process going away mid-trade, not an exception to
catch, which is why `detection_is_total` is a property test.

## Correctness rules

- **Rounding always favours the pessimistic reading.** Payouts floor, effective
  prices ceil, stakes round down to a tradeable increment. Every number the
  system reports should be one you can beat, not one you must hit.
- **Fees go in before the comparison, never after.** A raw-price detector
  produces a signal stream dominated by trades that were never profitable.
- **A lost sequence number means the book is out of service.** Never
  interpolate a gap; mark stale, reconnect, wait for a snapshot.
- **No arbitrage is not an error.** `Ok(None)` is the common case. `ArbError`
  is for malformed input only.
- **`detect` cannot tell whether the legs belong together.** Two prices from
  different games sum to whatever they like and look like a huge edge. That
  check is `arbkit-match`'s job and it is the most dangerous assumption in the
  system.

## Checks

```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run --example pipeline --release
```

Benchmark results are host-specific. Preserve prior results as dated baselines
and add a comparison column for a new host; do not silently replace measurements
from another architecture.
