//! TradeLedger: per-trade proof of trading accuracy.
//!
//! Renders a validated [`TradeLog`] as three zones — summary cards, an
//! expected-vs-realized chart, and an auditable table. Every displayed number
//! comes from `tradeMetrics` or straight off the record; nothing is
//! recomputed in JSX, so the pessimistic integers that survived the pipeline
//! are exactly what the user sees.
//!
//! Honesty rules (ROADMAP-TRADE-LEDGER invariant 3): `log === null` means no
//! ledger exists for this run and says so; zero records means the detector
//! found nothing and says that instead. Neither state fabricates rows.

import { useMemo, useState } from "react";
import {
  CartesianGrid,
  ReferenceLine,
  ResponsiveContainer,
  Scatter,
  ScatterChart,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { money, percent } from "../data/metrics";
import type { TradeRecord } from "../data/schema";
import type { TradeLog } from "../data/trades";
import { tradeMetrics } from "../data/tradeMetrics";

const CLASSIFICATIONS = ["clean", "proportional", "phantom", "brokenLeg"] as const;
type Classification = (typeof CLASSIFICATIONS)[number];

const CLASSIFICATION_LABELS: Record<Classification, string> = {
  clean: "Clean",
  proportional: "Proportional",
  phantom: "Phantom",
  brokenLeg: "Broken leg",
};

const PAGE_SIZE = 200;

const GREEN = "#2f6d43";
const COPPER = "#9a4d2f";

type SortKey = "edgeBps" | "realizedProfitCents" | "deltaCents";

export function TradeLedger({ log, error }: { log: TradeLog | null; error?: string }) {
  if (error) {
    return (
      <div className="trade-ledger" role="alert">
        <p className="empty-inline">
          The trade log could not be loaded ({error}). Check the recorded artifact and try again.
        </p>
      </div>
    );
  }

  if (!log) {
    return (
      <div className="trade-ledger">
        <p className="empty-inline">No trade log was recorded for this run.</p>
        <p className="method-note">
          Per-trade ledgers exist only for runs recorded with the ledger-enabled pipeline.
        </p>
      </div>
    );
  }

  if (log.records.length === 0) {
    return (
      <div className="trade-ledger">
        <p className="empty-inline">
          The detector found no arbitrage in this run, so there are no trades to audit.
        </p>
      </div>
    );
  }

  return <TradeLedgerBody log={log} />;
}

function TradeLedgerBody({ log }: { log: TradeLog }) {
  const metrics = useMemo(() => tradeMetrics(log.records), [log.records]);

  const [activeClasses, setActiveClasses] = useState<Set<Classification>>(
    () => new Set(CLASSIFICATIONS),
  );
  const [profitableOnly, setProfitableOnly] = useState(false);
  const [sortKey, setSortKey] = useState<SortKey>("realizedProfitCents");
  const [sortDescending, setSortDescending] = useState(true);
  const [page, setPage] = useState(0);

  const filtered = useMemo(() => {
    const rows = log.records.filter(
      (record) =>
        activeClasses.has(record.classification) &&
        (!profitableOnly || record.realizedProfitCents > 0),
    );
    const direction = sortDescending ? -1 : 1;
    return [...rows].sort((a, b) => {
      const deltaA = a.realizedProfitCents - a.expectedProfitCents;
      const deltaB = b.realizedProfitCents - b.expectedProfitCents;
      const left = sortKey === "edgeBps" ? a.edgeBps : sortKey === "realizedProfitCents" ? a.realizedProfitCents : deltaA;
      const right = sortKey === "edgeBps" ? b.edgeBps : sortKey === "realizedProfitCents" ? b.realizedProfitCents : deltaB;
      return (left - right) * direction;
    });
  }, [log.records, activeClasses, profitableOnly, sortKey, sortDescending]);

  const pageCount = Math.max(1, Math.ceil(filtered.length / PAGE_SIZE));
  const safePage = Math.min(page, pageCount - 1);
  const visible = filtered.slice(safePage * PAGE_SIZE, (safePage + 1) * PAGE_SIZE);

  const toggleSort = (key: SortKey) => {
    if (key === sortKey) {
      setSortDescending((descending) => !descending);
    } else {
      setSortKey(key);
      setSortDescending(true);
    }
  };

  const toggleClass = (classification: Classification) => {
    setPage(0);
    setActiveClasses((current) => {
      const next = new Set(current);
      if (next.has(classification)) {
        next.delete(classification);
      } else {
        next.add(classification);
      }
      return next.size > 0 ? next : new Set(CLASSIFICATIONS);
    });
  };

  return (
    <div className="trade-ledger">
      <div className="financial-ledger trade-summary" role="group" aria-label="Trade accuracy summary">
        <div>
          <span>Hit rate</span>
          <strong>{percent(metrics.hitRateBps)}</strong>
        </div>
        <div>
          <span>Expected vs realized</span>
          <strong>
            {money(metrics.totalExpectedCents)} → {money(metrics.totalRealizedCents)}
          </strong>
        </div>
        <div>
          <span>Median slippage</span>
          <strong>{money(metrics.medianSlippageCents)}</strong>
        </div>
        <div>
          <span>Phantom rate</span>
          <strong>{percent(metrics.phantomRateBps)}</strong>
          <small>{percent(metrics.cleanShareBps)} clean share</small>
        </div>
      </div>

      <ExpectedVsRealizedChart records={log.records} />

      <div className="trade-controls" role="group" aria-label="Trade filters">
        {CLASSIFICATIONS.map((classification) => (
          <button
            key={classification}
            type="button"
            className={`trade-chip ${activeClasses.has(classification) ? "is-active" : ""}`}
            aria-pressed={activeClasses.has(classification)}
            onClick={() => toggleClass(classification)}
          >
            {CLASSIFICATION_LABELS[classification]}
          </button>
        ))}
        <button
          type="button"
          className={`trade-chip ${profitableOnly ? "is-active" : ""}`}
          aria-pressed={profitableOnly}
          onClick={() => {
            setPage(0);
            setProfitableOnly((value) => !value);
          }}
        >
          Profitable only
        </button>
      </div>

      <p className="method-note trade-count" aria-live="polite">
        Showing {visible.length} of {filtered.length} matching trades
        {filtered.length !== log.records.length ? ` (${log.records.length} total)` : ""}.
      </p>

      <div className="history-table-wrap">
        <table className="history-table trade-table">
          <thead>
            <tr>
              <th scope="col">#</th>
              <th scope="col">Market</th>
              <th scope="col" aria-sort={sortHeader(sortKey, sortDescending, "edgeBps")}>
                <button type="button" onClick={() => toggleSort("edgeBps")}>Edge</button>
              </th>
              <th scope="col">Stake</th>
              <th scope="col">Expected</th>
              <th scope="col" aria-sort={sortHeader(sortKey, sortDescending, "realizedProfitCents")}>
                <button type="button" onClick={() => toggleSort("realizedProfitCents")}>Realized</button>
              </th>
              <th scope="col" aria-sort={sortHeader(sortKey, sortDescending, "deltaCents")}>
                <button type="button" onClick={() => toggleSort("deltaCents")} aria-label="Delta, realized minus expected">Δ</button>
              </th>
              <th scope="col">Class</th>
              <th scope="col">Detail</th>
            </tr>
          </thead>
          <tbody>
            {visible.map((record) => (
              <TradeRow key={record.seq} record={record} />
            ))}
          </tbody>
        </table>
      </div>

      {pageCount > 1 && (
        <nav className="trade-pagination" aria-label="Trade pages">
          <button
            type="button"
            disabled={safePage === 0}
            onClick={() => setPage(safePage - 1)}
          >
            Previous
          </button>
          <span>
            Page {safePage + 1} / {pageCount}
          </span>
          <button
            type="button"
            disabled={safePage >= pageCount - 1}
            onClick={() => setPage(safePage + 1)}
          >
            Next
          </button>
        </nav>
      )}
    </div>
  );
}

function sortHeader(activeKey: SortKey, descending: boolean, key: SortKey): "ascending" | "descending" | "none" {
  if (key !== activeKey) return "none";
  return descending ? "descending" : "ascending";
}

function TradeRow({ record }: { record: TradeRecord }) {
  const [expanded, setExpanded] = useState(false);
  const delta = record.realizedProfitCents - record.expectedProfitCents;

  return (
    <>
      <tr className={record.realizedProfitCents > 0 ? undefined : "is-loss"}>
        <th scope="row">{record.seq}</th>
        <td>{record.marketLabel}</td>
        <td>{record.edgeBps} bps</td>
        <td>{money(record.requestedStakeCents)}</td>
        <td>{money(record.expectedProfitCents)}</td>
        <td>{money(record.realizedProfitCents)}</td>
        <td>{delta >= 0 ? "+" : ""}{money(delta).replace("$-", "-$")}</td>
        <td>
          <span className={`trade-badge trade-badge--${record.classification}`} data-classification={record.classification}>
            {CLASSIFICATION_LABELS[record.classification]}
          </span>
          {record.chased && <span className="trade-badge trade-badge--chased">Chased</span>}
        </td>
        <td>
          <button
            type="button"
            aria-expanded={expanded}
            onClick={() => setExpanded((value) => !value)}
          >
            {expanded ? "Hide legs" : "Show legs"}
          </button>
        </td>
      </tr>
      {expanded && <LegAuditRow record={record} />}
    </>
  );
}

function LegAuditRow({ record }: { record: TradeRecord }) {
  return (
    <tr className="trade-legs-row">
      <td colSpan={9}>
        <dl className="trade-legs">
          {record.legs.map((leg, index) => (
            <div key={index}>
              <dt>
                {leg.venueLabel} · {leg.outcomeLabel}
              </dt>
              <dd>
                {describeStatus(leg.status)} · requested {money(leg.requestedStakeCents)}, filled{" "}
                {money(leg.filledStakeCents)}, net payout {money(leg.netPayoutCents)}
              </dd>
            </div>
          ))}
        </dl>
      </td>
    </tr>
  );
}

function describeStatus(status: TradeRecord["legs"][number]["status"]): string {
  if (status === "filled") return "filled";
  if ("partiallyFilled" in status) {
    const detail = status.partiallyFilled;
    return `partially filled (${detail.reason}: ${money(detail.filledCents)} of stake filled, ${money(detail.unfilledCents)} unfilled)`;
  }
  return `unfilled (${status.unfilled})`;
}

function ExpectedVsRealizedChart({ records }: { records: TradeRecord[] }) {
  const points = records.map((record) => ({
    expected: record.expectedProfitCents,
    realized: record.realizedProfitCents,
  }));
  const bounds = points.reduce(
    (acc, point) => ({
      min: Math.min(acc.min, point.expected, point.realized),
      max: Math.max(acc.max, point.expected, point.realized),
    }),
    { min: Number.POSITIVE_INFINITY, max: Number.NEGATIVE_INFINITY },
  );

  return (
    <figure className="chart-figure">
      <div className="chart-frame chart-frame--trades">
        <ResponsiveContainer width="100%" height="100%">
          <ScatterChart margin={{ top: 16, right: 24, bottom: 12, left: 8 }} accessibilityLayer>
            <CartesianGrid stroke="#d7d0c0" strokeDasharray="2 5" />
            <XAxis
              type="number"
              dataKey="expected"
              name="Expected"
              tickFormatter={(value) => `${Number(value).toLocaleString()}c`}
              tickLine={false}
              axisLine={{ stroke: "#171814" }}
            />
            <YAxis
              type="number"
              dataKey="realized"
              name="Realized"
              tickFormatter={(value) => `${Number(value).toLocaleString()}c`}
              tickLine={false}
              axisLine={false}
              width={80}
            />
            <Tooltip
              cursor={{ strokeDasharray: "4 4" }}
              formatter={(value, name) => [`${Number(value).toLocaleString()} cents`, String(name)]}
              contentStyle={{
                background: "#f3efe5",
                border: "1px solid #171814",
                borderRadius: 0,
                fontFamily: "IBM Plex Mono, monospace",
                fontSize: 12,
              }}
            />
            {/* The y=x diagonal: anything below it failed to realize its
                detected edge, making slippage losses visually obvious. */}
            <ReferenceLine
              segment={[
                { x: bounds.min, y: bounds.min },
                { x: bounds.max, y: bounds.max },
              ]}
              stroke={GREEN}
              strokeDasharray="6 5"
              label={{ value: "expected = realized", fill: GREEN, position: "insideTopLeft" }}
            />
            <ReferenceLine y={0} stroke={COPPER} strokeDasharray="4 4" />
            <Scatter name="Trades" data={points} fill="#315a78" fillOpacity={0.55} />
          </ScatterChart>
        </ResponsiveContainer>
      </div>
      <figcaption>
        Each point is one trade: below the diagonal means the realized outcome fell short of the
        fee-adjusted detection view; below zero means the trade lost money.
      </figcaption>
      <details className="data-table">
        <summary>View expected-vs-realized data table</summary>
        <table>
          <thead>
            <tr>
              <th scope="col">Trade #</th>
              <th scope="col">Expected (cents)</th>
              <th scope="col">Realized (cents)</th>
            </tr>
          </thead>
          <tbody>
            {records.map((record) => (
              <tr key={record.seq}>
                <th scope="row">{record.seq}</th>
                <td>{record.expectedProfitCents.toLocaleString()}</td>
                <td>{record.realizedProfitCents.toLocaleString()}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </details>
    </figure>
  );
}
