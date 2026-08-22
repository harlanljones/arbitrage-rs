//! Pure accuracy metrics over validated trade records (ROADMAP-TRADE-LEDGER
//! §2.3). No fetching, no React, no recomputation of profit: every input is a
//! validated integer-cents value and every output is derived from those
//! integers with integer-safe arithmetic only.

import type { TradeRecord } from "./schema";

/** Guard against sums overflowing the exact-integer range of JS numbers.
 * Realistic logs are thousands of records; 2^31 is astronomically past that. */
const MAX_RECORDS = 2 ** 31;

export function tradeMetrics(records: TradeRecord[]): {
  hitRateBps: number;
  totalExpectedCents: number;
  totalRealizedCents: number;
  medianSlippageCents: number;
  phantomRateBps: number;
  cleanShareBps: number;
} {
  const total = records.length;
  if (total >= MAX_RECORDS) {
    throw new RangeError(`Trade count ${total} exceeds the ${MAX_RECORDS} safe bound.`);
  }

  // Empty input is a valid state (a run with no detected arbitrage), so every
  // rate is defined as zero rather than dividing by zero.
  if (total === 0) {
    return {
      hitRateBps: 0,
      totalExpectedCents: 0,
      totalRealizedCents: 0,
      medianSlippageCents: 0,
      phantomRateBps: 0,
      cleanShareBps: 0,
    };
  }

  let hits = 0;
  let phantoms = 0;
  let clean = 0;
  let totalExpectedCents = 0;
  let totalRealizedCents = 0;

  for (const record of records) {
    if (record.realizedProfitCents > 0) hits += 1;
    if (record.classification === "phantom" || record.classification === "brokenLeg") phantoms += 1;
    if (record.classification === "clean") clean += 1;
    totalExpectedCents += record.expectedProfitCents;
    totalRealizedCents += record.realizedProfitCents;
  }

  return {
    hitRateBps: Math.floor((hits * 10_000) / total),
    totalExpectedCents,
    totalRealizedCents,
    // Median of integer cents; an even-count average may halve, so it floors
    // — the pessimistic reading of slippage, never flattering the result.
    medianSlippageCents: median(records.map((record) => record.slippageCents)),
    phantomRateBps: Math.floor((phantoms * 10_000) / total),
    cleanShareBps: Math.floor((clean * 10_000) / total),
  };
}

function median(values: number[]): number {
  const sorted = [...values].sort((a, b) => a - b);
  const middle = sorted.length >> 1;
  if (sorted.length % 2 === 1) return sorted[middle];
  return Math.floor((sorted[middle - 1] + sorted[middle]) / 2);
}
