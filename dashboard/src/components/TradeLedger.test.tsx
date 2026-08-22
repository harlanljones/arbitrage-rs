import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { TradeLedger } from "./TradeLedger";
import type { TradeRecord } from "../data/schema";
import type { TradeLog } from "../data/trades";
import { tradeMetrics } from "../data/tradeMetrics";

const clean: TradeRecord = {
  seq: 0,
  detectionTimestampNs: 1_000,
  latencyNs: 200,
  marketLabel: "BOS @ LAL · moneyline",
  edgeBps: 300,
  overroundPpm: 960_000,
  requestedStakeCents: 10_000,
  expectedProfitCents: 300,
  worstCaseProfitCents: 300,
  realizedProfitCents: 250,
  slippageCents: 50,
  feesPaidCents: 40,
  fillRatioBps: 10_000,
  classification: "clean",
  chased: false,
  legs: [
    {
      venueLabel: "polymarket",
      outcomeLabel: "Lakers",
      status: "filled",
      requestedStakeCents: 5_000,
      filledStakeCents: 5_000,
      netPayoutCents: 10_100,
    },
    {
      venueLabel: "kalshi",
      outcomeLabel: "Celtics",
      status: {
        partiallyFilled: { filledCents: 4_900, unfilledCents: 100, reason: "depthDepleted" },
      },
      requestedStakeCents: 5_000,
      filledStakeCents: 4_900,
      netPayoutCents: 9_800,
    },
  ],
};

const phantom: TradeRecord = {
  ...clean,
  seq: 1,
  edgeBps: 120,
  expectedProfitCents: 150,
  realizedProfitCents: -5_000,
  slippageCents: 5_150,
  classification: "phantom",
  legs: [
    {
      venueLabel: "polymarket",
      outcomeLabel: "Lakers",
      status: { unfilled: "priceMoved" },
      requestedStakeCents: 5_000,
      filledStakeCents: 0,
      netPayoutCents: 0,
    },
    {
      venueLabel: "kalshi",
      outcomeLabel: "Celtics",
      status: "filled",
      requestedStakeCents: 5_000,
      filledStakeCents: 5_000,
      netPayoutCents: 9_700,
    },
  ],
};

const log: TradeLog = {
  header: { schemaVersion: 1, kind: "arbkit-trades", runId: "r", tradeCount: 2 },
  records: [clean, phantom],
};

afterEach(() => cleanup());

describe("TradeLedger states", () => {
  it("shows an honest empty state when no ledger exists", () => {
    render(<TradeLedger log={null} />);
    expect(screen.getByText(/no trade log was recorded/i)).toBeInTheDocument();
  });

  it("distinguishes a zero-trade run from missing data", () => {
    render(
      <TradeLedger
        log={{
          header: { schemaVersion: 1, kind: "arbkit-trades", runId: "r", tradeCount: 0 },
          records: [],
        }}
      />,
    );
    expect(screen.getByText(/found no arbitrage/i)).toBeInTheDocument();
  });

  it("surfaces loader errors with a retry hint", () => {
    render(<TradeLedger log={null} error="returned 500" />);
    expect(screen.getByRole("alert")).toHaveTextContent(/could not be loaded/);
    expect(screen.getByRole("alert")).toHaveTextContent(/try again/);
  });
});

describe("TradeLedger summary and table", () => {
  it("renders summary cards matching tradeMetrics exactly", () => {
    render(<TradeLedger log={log} />);
    const metrics = tradeMetrics(log.records);
    expect(metrics.hitRateBps).toBe(5_000);
    // Hit rate and phantom rate land on 50.00%; clean share is folded into
    // the phantom card as a qualifier.
    expect(screen.getAllByText("50.00%")).toHaveLength(2);
    expect(screen.getByText(/50\.00% clean share/)).toBeInTheDocument();
    // expected total 450c -> $4.50 ; realized total -4750c -> -$47.50
    expect(screen.getByText("$4.50 → -$47.50")).toBeInTheDocument();
  });

  it("starts with the per-trade table collapsed to keep the page short", () => {
    render(<TradeLedger log={log} />);
    const toggle = screen.getByRole("button", { name: /inspect the per-trade table/i });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(document.querySelector("table.trade-table")).toBeNull();
  });

  function openTradeTable() {
    fireEvent.click(screen.getByRole("button", { name: /inspect the per-trade table/i }));
    return document.querySelector("table.trade-table") as HTMLElement;
  }

  it("filters by classification chip and reports the count live", () => {
    render(<TradeLedger log={log} />);
    expect(screen.getByRole("button", { name: /inspect the per-trade table \(2\)/i })).toBeInTheDocument();

    openTradeTable();
    expect(screen.getByText(/showing 2 of 2/i)).toBeInTheDocument();

    // Removing the phantom chip leaves only the profitable clean trade.
    fireEvent.click(screen.getByRole("button", { name: "Phantom" }));
    expect(screen.getByText(/showing 1 of 1 matching trades \(2 total\)/i)).toBeInTheDocument();
    // The chart's data table also has role="table"; scope to the trade table.
    const table = document.querySelector("table.trade-table") as HTMLElement;
    expect(table).toHaveTextContent("300 bps");
    expect(table).not.toHaveTextContent("-$50.00");
  });

  it("toggles profitable-only and hides losing trades", async () => {
    render(<TradeLedger log={log} />);
    openTradeTable();
    const chip = screen.getByRole("button", { name: "Profitable only" });
    expect(chip).toHaveAttribute("aria-pressed", "false");
    fireEvent.click(chip);
    expect(chip).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByText(/showing 1 of 1 matching trades \(2 total\)/i)).toBeInTheDocument();
    expect(screen.queryByText("-$50.00")).not.toBeInTheDocument();
  });

  it("sorts by realized profit ascending on second click", async () => {
    render(<TradeLedger log={log} />);
    openTradeTable();
    const tradeTable = document.querySelector("table.trade-table") as HTMLElement;
    // The chart's data table also contributes rows; scope to the trade table.
    const rows = () =>
      Array.from(tradeTable.querySelectorAll("tbody tr")).map((row) => row.textContent ?? "");

    fireEvent.click(screen.getByRole("button", { name: "Realized" })); // descending first click
    expect(rows()[0]).toContain("-$50.00");
    fireEvent.click(screen.getByRole("button", { name: "Realized" })); // ascending second click
    expect(rows()[0]).toContain("$2.50");
  });

  it("expands a row into its per-leg audit with statuses and reasons", async () => {
    render(<TradeLedger log={log} />);
    openTradeTable();

    const expanders = screen.getAllByRole("button", { name: /show legs/i });
    expect(expanders).toHaveLength(2);
    expect(expanders[0]).toHaveAttribute("aria-expanded", "false");

    fireEvent.click(expanders[0]);
    expect(expanders[0]).toHaveAttribute("aria-expanded", "true");
    expect(screen.getAllByText(/filled · requested/).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/partially filled \(depthDepleted:/).length).toBeGreaterThan(0);

    fireEvent.click(screen.getAllByRole("button", { name: /show legs/i })[0]);
    expect(screen.getAllByText(/unfilled \(priceMoved\)/).length).toBeGreaterThan(0);
  });
});
