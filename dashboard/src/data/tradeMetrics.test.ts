import { describe, expect, it, vi } from "vitest";
import {
  TradeLogHeaderSchema,
  TradeRecordSchema,
  type RunSnapshot,
  type TradeRecord,
} from "./schema";
import { loadTradeLog } from "./trades";
import { tradeMetrics } from "./tradeMetrics";

// One fixture line per leg-status variant, covering every classification.
const cleanRecord: TradeRecord = {
  seq: 0,
  detectionTimestampNs: 75_715,
  latencyNs: 141_004,
  marketLabel: "Boston Celtics @ Los Angeles Lakers · moneyline",
  edgeBps: 28,
  overroundPpm: 997_150,
  requestedStakeCents: 98_310,
  expectedProfitCents: 280,
  worstCaseProfitCents: 280,
  realizedProfitCents: 190,
  slippageCents: 90,
  feesPaidCents: 120,
  fillRatioBps: 10_000,
  classification: "clean",
  chased: false,
  legs: [
    {
      venueLabel: "polymarket",
      outcomeLabel: "Los Angeles Lakers",
      status: "filled",
      requestedStakeCents: 48_310,
      filledStakeCents: 48_310,
      netPayoutCents: 100_650,
    },
    {
      venueLabel: "kalshi",
      outcomeLabel: "Boston Celtics",
      status: "filled",
      requestedStakeCents: 50_000,
      filledStakeCents: 50_000,
      netPayoutCents: 98_150,
    },
  ],
};

const proportionalRecord: TradeRecord = {
  ...cleanRecord,
  seq: 1,
  expectedProfitCents: 300,
  realizedProfitCents: 140,
  slippageCents: 160,
  classification: "proportional",
  legs: [
    {
      venueLabel: "polymarket",
      outcomeLabel: "Los Angeles Lakers",
      status: {
        partiallyFilled: { filledCents: 40_000, unfilledCents: 8_310, reason: "depthDepleted" },
      },
      requestedStakeCents: 48_310,
      filledStakeCents: 40_000,
      netPayoutCents: 83_330,
    },
    {
      venueLabel: "kalshi",
      outcomeLabel: "Boston Celtics",
      status: {
        partiallyFilled: {
          filledCents: 50_000,
          unfilledCents: 1_000,
          reason: "incrementRounding",
        },
      },
      requestedStakeCents: 50_000,
      filledStakeCents: 50_000,
      netPayoutCents: 98_150,
    },
  ],
};

const phantomRecord: TradeRecord = {
  ...cleanRecord,
  seq: 2,
  expectedProfitCents: 280,
  realizedProfitCents: -48_310,
  slippageCents: 48_590,
  feesPaidCents: 0,
  fillRatioBps: 5_086,
  classification: "phantom",
  legs: [
    {
      venueLabel: "polymarket",
      outcomeLabel: "Los Angeles Lakers",
      status: "filled",
      requestedStakeCents: 48_310,
      filledStakeCents: 48_310,
      netPayoutCents: 100_650,
    },
    {
      venueLabel: "kalshi",
      outcomeLabel: "Boston Celtics",
      status: { unfilled: "priceMoved" },
      requestedStakeCents: 50_000,
      filledStakeCents: 0,
      netPayoutCents: 0,
    },
  ],
};

const brokenLegRecord: TradeRecord = {
  ...cleanRecord,
  seq: 3,
  expectedProfitCents: 260,
  realizedProfitCents: -50_000,
  slippageCents: 50_260,
  classification: "brokenLeg",
  chased: true,
  legs: [
    {
      venueLabel: "polymarket",
      outcomeLabel: "Los Angeles Lakers",
      status: { unfilled: "bookStale" },
      requestedStakeCents: 48_310,
      filledStakeCents: 0,
      netPayoutCents: 0,
    },
    {
      venueLabel: "kalshi",
      outcomeLabel: "Boston Celtics",
      status: "filled",
      requestedStakeCents: 50_000,
      filledStakeCents: 50_000,
      netPayoutCents: 98_150,
    },
  ],
};

const validRecords = [cleanRecord, proportionalRecord, phantomRecord, brokenLegRecord];

const header = {
  schemaVersion: 1,
  kind: "arbkit-trades" as const,
  runId: "test-run",
  tradeCount: validRecords.length,
};

const run = {
  run: { id: "test-run", source: "measured" as const, projectVersion: "0.1.0" },
} as unknown as RunSnapshot;

function tradesBody(): string {
  return [JSON.stringify(header), ...validRecords.map((r) => JSON.stringify(r))].join("\n") + "\n";
}

describe("trade log schemas", () => {
  it("accepts the frozen wire format", () => {
    expect(TradeLogHeaderSchema.parse(header)).toEqual(header);
    for (const record of validRecords) {
      expect(TradeRecordSchema.parse(record)).toEqual(record);
    }
  });

  it("rejects a wrong schemaVersion or kind", () => {
    expect(() => TradeLogHeaderSchema.parse({ ...header, schemaVersion: 2 })).toThrow();
    expect(() => TradeLogHeaderSchema.parse({ ...header, kind: "something-else" })).toThrow();
  });

  it("rejects float money anywhere in a record", () => {
    expect(() =>
      TradeRecordSchema.parse({ ...cleanRecord, realizedProfitCents: 190.5 }),
    ).toThrow();
    expect(() =>
      TradeRecordSchema.parse({
        ...cleanRecord,
        legs: [{ ...cleanRecord.legs[0], filledStakeCents: 0.25 }, ...cleanRecord.legs.slice(1)],
      }),
    ).toThrow();
  });

  it("rejects an unknown classification", () => {
    expect(() =>
      TradeRecordSchema.parse({ ...cleanRecord, classification: "lucky" }),
    ).toThrow();
  });

  it("rejects more legs than MAX_LEGS", () => {
    const fiveLegs = Array.from({ length: 5 }, () => ({ ...cleanRecord.legs[0] }));
    expect(() => TradeRecordSchema.parse({ ...cleanRecord, legs: fiveLegs })).toThrow();
  });
});

describe("loadTradeLog", () => {
  it("returns null when the run has no trade log (404)", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response("not found", { status: 404 })),
    );
    await expect(loadTradeLog(run)).resolves.toBeNull();
    vi.unstubAllGlobals();
  });

  it("parses a well-formed log and validates the header count", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response(tradesBody(), { status: 200 })),
    );
    const log = await loadTradeLog(run);
    expect(log).not.toBeNull();
    expect(log?.header.tradeCount).toBe(4);
    expect(log?.records).toHaveLength(4);
    vi.unstubAllGlobals();
  });

  it("throws when the header count disagrees with the record lines", async () => {
    const body =
      [JSON.stringify({ ...header, tradeCount: 99 }), JSON.stringify(cleanRecord)].join("\n") + "\n";
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response(body, { status: 200 })),
    );
    await expect(loadTradeLog(run)).rejects.toThrow(/counts 99/);
    vi.unstubAllGlobals();
  });

  it("throws when a record violates the schema", async () => {
    const body =
      [JSON.stringify(header), JSON.stringify({ ...cleanRecord, edgeBps: 28.5 })].join("\n") + "\n";
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response(body, { status: 200 })),
    );
    await expect(loadTradeLog(run)).rejects.toThrow(/failed validation/);
    vi.unstubAllGlobals();
  });

  it("throws when the header names a different run", async () => {
    const body =
      [JSON.stringify({ ...header, runId: "other-run" }), JSON.stringify(cleanRecord)].join("\n") +
      "\n";
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response(body, { status: 200 })),
    );
    await expect(loadTradeLog(run)).rejects.toThrow(/claims run other-run/);
    vi.unstubAllGlobals();
  });
});

describe("tradeMetrics", () => {
  it("returns zeros without dividing for an empty ledger", () => {
    expect(tradeMetrics([])).toEqual({
      hitRateBps: 0,
      totalExpectedCents: 0,
      totalRealizedCents: 0,
      medianSlippageCents: 0,
      phantomRateBps: 0,
      cleanShareBps: 0,
    });
  });

  it("computes hand-checked aggregates over every classification", () => {
    const metrics = tradeMetrics(validRecords);
    // hits: clean(190) + proportional(140) > 0 -> 2 of 4
    expect(metrics.hitRateBps).toBe(5_000);
    expect(metrics.totalExpectedCents).toBe(280 + 300 + 280 + 260);
    expect(metrics.totalRealizedCents).toBe(190 + 140 - 48_310 - 50_000);
    // phantom + brokenLeg -> 2 of 4
    expect(metrics.phantomRateBps).toBe(5_000);
    expect(metrics.cleanShareBps).toBe(2_500);
  });

  it("floors the median of an even count of slippages", () => {
    const metrics = tradeMetrics([
      { ...cleanRecord, slippageCents: 11 },
      { ...proportionalRecord, slippageCents: 14 },
    ]);
    expect(metrics.medianSlippageCents).toBe(12);
  });

  it("returns the middle value unchanged for an odd count", () => {
    const metrics = tradeMetrics([
      { ...cleanRecord, slippageCents: 30 },
      { ...proportionalRecord, slippageCents: 10 },
      { ...phantomRecord, slippageCents: 20 },
    ]);
    expect(metrics.medianSlippageCents).toBe(20);
  });
});
