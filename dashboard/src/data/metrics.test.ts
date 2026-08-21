import { describe, expect, it } from "vitest";
import { executionRate, headroom, nsToMicros, throughputDelta } from "./metrics";
import { RunSnapshotSchema } from "./schema";

const snapshot = RunSnapshotSchema.parse({
  schemaVersion: 1,
  run: { id: "test", recordedAt: "2026-08-21", source: "measured", projectVersion: "0.1.0" },
  environment: { label: "test host", os: "linux", arch: "x86_64", buildProfile: "release" },
  workload: {
    synthetic: true,
    paperTrading: true,
    feedEvents: 200000,
    event: "BOS @ LAL",
    market: "Moneyline",
    venues: ["Kalshi", "Polymarket"],
  },
  performance: {
    elapsedMs: 31.5,
    throughputPerSecond: 6000000,
    targetP99Ns: 50000,
    latencyNs: { min: 90, mean: 97, p50: 90, p90: 100, p99: 100, p999: 480, max: 486 },
  },
  detection: { eventsProcessed: 200004, signalsEmitted: 829, collectedSignals: 829 },
  simulation: {
    totalSignals: 829,
    cleanFills: 0,
    proportionalFills: 746,
    phantoms: 83,
    phantomRateBps: 1001,
    filledStakeCents: 72876755,
    feesPaidCents: 2605032,
    realizedProfitCents: 1550173,
    realizedRoiBps: 212,
  },
});

describe("derived benchmark metrics", () => {
  it("converts and compares without rounding away evidence", () => {
    expect(nsToMicros(100)).toBe(0.1);
    expect(headroom(snapshot)).toBe(500);
    expect(executionRate(snapshot)).toBeCloseTo(89.9879, 3);
  });

  it("computes host comparison deltas", () => {
    const baseline = {
      ...snapshot,
      performance: { ...snapshot.performance, throughputPerSecond: 3000000 },
    };
    expect(throughputDelta(snapshot, baseline)).toBe(100);
  });

  it("rejects snapshots that omit provenance-critical fields", () => {
    const invalid = { ...snapshot, workload: { ...snapshot.workload, synthetic: undefined } };
    expect(() => RunSnapshotSchema.parse(invalid)).toThrow();
  });
});
