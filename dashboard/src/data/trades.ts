//! Loader for the per-trade accuracy ledger (ROADMAP-TRADE-LEDGER §2.3).
//!
//! Trade logs are static JSONL assets under `data/runs/`, fetched lazily so a
//! large ledger never bloats the bundle. The recorder publishes each log as
//! `<run-id>.trades.jsonl`, which is how the file is located here: a
//! `RunSnapshot` does not carry its own index filename, but the run id it does
//! carry is exactly what the recorder used.
//!
//! Absence (`null`) is a normal, honest outcome for pre-ledger runs. Schema
//! violations throw — a corrupt proof must never render as "no data".

import {
  TradeLogHeaderSchema,
  TradeRecordSchema,
  type RunSnapshot,
  type TradeLogHeader,
  type TradeRecord,
} from "./schema";

/** Defensive upper bound on an accepted trade log; the recorder's runs stay
 * orders of magnitude below this, so exceeding it means corruption or an
 * accident, not data worth rendering. */
const MAX_TRADES_BYTES = 20 * 1024 * 1024;

export type TradeLog = { header: TradeLogHeader; records: TradeRecord[] };

export async function loadTradeLog(run: RunSnapshot): Promise<TradeLog | null> {
  const dataRoot = `${import.meta.env.BASE_URL}data/runs/`;
  const response = await fetch(`${dataRoot}${run.run.id}.trades.jsonl`);
  if (response.status === 404) return null;
  if (!response.ok) {
    throw new Error(`The trade log for run ${run.run.id} returned ${response.status}.`);
  }

  const declaredLength = Number(response.headers.get("content-length") ?? "0");
  if (declaredLength > MAX_TRADES_BYTES) {
    throw new Error(
      `The trade log for run ${run.run.id} exceeds the ${MAX_TRADES_BYTES}-byte safety cap.`,
    );
  }
  const text = await response.text();
  if (text.length > MAX_TRADES_BYTES) {
    throw new Error(
      `The trade log for run ${run.run.id} exceeds the ${MAX_TRADES_BYTES}-byte safety cap.`,
    );
  }

  const lines = text.split("\n").filter((line) => line.trim().length > 0);
  if (lines.length === 0) {
    throw new Error(`The trade log for run ${run.run.id} is empty.`);
  }

  // Header first, then every record — fail loudly on any mismatch rather
  // than rendering a partial proof as if it were complete.
  const header = TradeLogHeaderSchema.parse(JSON.parse(lines[0]));
  if (header.runId !== run.run.id) {
    throw new Error(
      `Trade log header claims run ${header.runId}, expected ${run.run.id}.`,
    );
  }

  const records: TradeRecord[] = lines.slice(1).map((line, index) => {
    try {
      return TradeRecordSchema.parse(JSON.parse(line));
    } catch (cause: unknown) {
      throw new Error(
        `Trade record ${index + 1} of ${run.run.id} failed validation: ${
          cause instanceof Error ? cause.message : String(cause)
        }`,
      );
    }
  });

  if (records.length !== header.tradeCount) {
    throw new Error(
      `Trade log header counts ${header.tradeCount} trades but ${records.length} lines follow.`,
    );
  }

  return { header, records };
}
