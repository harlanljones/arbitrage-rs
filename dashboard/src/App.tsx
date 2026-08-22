import { lazy, Suspense, useEffect, useMemo, useState } from "react";
import { BudgetRuler } from "./components/BudgetRuler";
import {
  compact,
  executionRate,
  formatDate,
  headroom,
  money,
  nsToMicros,
  percent,
  throughputDelta,
} from "./data/metrics";
import { loadRunHistory, type RunSnapshot } from "./data/schema";
import { loadTradeLog, type TradeLog } from "./data/trades";

const LatencyProfileChart = lazy(() =>
  import("./components/Charts").then((module) => ({ default: module.LatencyProfileChart })),
);
const ThroughputChart = lazy(() =>
  import("./components/Charts").then((module) => ({ default: module.ThroughputChart })),
);
const VerificationChart = lazy(() =>
  import("./components/Charts").then((module) => ({ default: module.VerificationChart })),
);
const TradeLedger = lazy(() =>
  import("./components/TradeLedger").then((module) => ({ default: module.TradeLedger })),
);

function ChartFallback() {
  return <div className="chart-fallback" role="status">Preparing the evidence plot…</div>;
}

export function App() {
  const [runs, setRuns] = useState<RunSnapshot[]>([]);
  const [selectedId, setSelectedId] = useState("");
  const [status, setStatus] = useState<"loading" | "ready" | "error">("loading");
  const [error, setError] = useState("");
  const [tradeLog, setTradeLog] = useState<TradeLog | null>(null);
  const [tradeStatus, setTradeStatus] = useState<"loading" | "ready" | "error">("loading");
  const [tradeError, setTradeError] = useState("");

  const refresh = () => {
    setStatus("loading");
    setError("");
    loadRunHistory()
      .then((history) => {
        if (history.length === 0) throw new Error("No published benchmark runs were found.");
        setRuns(history);
        setSelectedId((current) => current || history[0].run.id);
        setStatus("ready");
      })
      .catch((cause: unknown) => {
        setError(cause instanceof Error ? cause.message : "The benchmark history could not be loaded.");
        setStatus("error");
      });
  };

  useEffect(refresh, []);

  const selected = useMemo(
    () => runs.find((run) => run.run.id === selectedId) ?? runs[0],
    [runs, selectedId],
  );

  // Trade logs are per-run artifacts: reset and refetch whenever the selected
  // run changes. A failure here must not block the rest of the page — it is
  // contained inside the trades section.
  const selectedRunId = selected?.run.id;
  useEffect(() => {
    if (!selectedRunId) return;
    let cancelled = false;
    setTradeStatus("loading");
    setTradeError("");
    setTradeLog(null);
    // The loader only reads `run.run.id`; the id alone identifies the file.
    const target = { run: { id: selectedRunId } } as RunSnapshot;
    loadTradeLog(target)
      .then((log) => {
        if (cancelled) return;
        setTradeLog(log);
        setTradeStatus("ready");
      })
      .catch((cause: unknown) => {
        if (cancelled) return;
        setTradeError(cause instanceof Error ? cause.message : "The trade log could not be loaded.");
        setTradeStatus("error");
      });
    return () => {
      cancelled = true;
    };
  }, [selectedRunId]);

  if (status === "loading") {
    return (
      <main className="state-page" aria-live="polite">
        <span className="state-rule" />
        <h1>Opening the results ledger.</h1>
        <p>Validating dated benchmark snapshots and their provenance…</p>
      </main>
    );
  }

  if (status === "error" || !selected) {
    return (
      <main className="state-page" role="alert">
        <span className="state-rule state-rule--error" />
        <h1>The evidence ledger could not open.</h1>
        <p>{error}</p>
        <button type="button" onClick={refresh}>Try loading the run history again</button>
      </main>
    );
  }

  const baseline = runs.find((run) => run.run.id !== selected.run.id) ?? selected;
  const verified = selected.verification;

  const downloadSnapshot = () => {
    const blob = new Blob([`${JSON.stringify(selected, null, 2)}\n`], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = `${selected.run.id}.json`;
    anchor.click();
    URL.revokeObjectURL(url);
  };

  return (
    <div className="site-shell">
      <aside className="binder-rail" aria-hidden="true">
        <span className="binder-hole" />
        <span className="binder-hole" />
        <span className="binder-hole" />
        <strong>Proof ledger</strong>
        <em>001</em>
      </aside>
      <header className="site-header">
        <a className="wordmark" href="#top" aria-label="arbkit results home">arbkit</a>
        <nav aria-label="Results sections">
          <a href="#performance">Performance</a>
          <a href="#execution">Execution</a>
          <a href="#trades">Trades</a>
          <a href="#verification">Verification</a>
          <a href="#history">History</a>
        </nav>
        <div className="scope-stamp">
          <strong>Synthetic workload</strong>
          <span>Paper trading · No live orders</span>
        </div>
      </header>

      <main id="top">
        <section className="hero" aria-labelledby="hero-heading">
          <div className="hero-main">
            <div className="run-control">
              <label htmlFor="run-select">Published run</label>
              <select id="run-select" value={selected.run.id} onChange={(event) => setSelectedId(event.target.value)}>
                {runs.map((run) => (
                  <option value={run.run.id} key={run.run.id}>
                    {formatDate(run)} · {run.environment.label}
                  </option>
                ))}
              </select>
            </div>
            <h1 id="hero-heading">
              <span className="hero-value">{nsToMicros(selected.performance.latencyNs.p99).toFixed(3)}</span>
              <span className="hero-unit">µs</span>
              <span className="hero-label">p99 hot loop</span>
            </h1>
            <p className="hero-thesis">
              The detector clears its in-process latency budget before the market data has time to blink.
            </p>
            <div className="hero-facts" aria-label="Latency result summary">
              <div><strong>{nsToMicros(selected.performance.targetP99Ns).toFixed(3)} µs</strong><span>budget</span></div>
              <div><strong>{headroom(selected).toFixed(0)}×</strong><span>headroom</span></div>
              <div><strong>{selected.performance.latencyNs.max} ns</strong><span>maximum observed</span></div>
            </div>
          </div>
          <BudgetRuler selected={selected} />
        </section>

        <section className="evidence-rail" aria-label="Project success summary">
          <div>
            <span>Ingest & match</span>
            <strong>{compact(selected.performance.throughputPerSecond)}</strong>
            <small>updates / second</small>
          </div>
          <div>
            <span>Signal disposition</span>
            <strong>{selected.simulation.proportionalFills}</strong>
            <small>proportional fills · {selected.simulation.phantoms} phantom</small>
          </div>
          <div>
            <span>Paper trading</span>
            <strong>{money(selected.simulation.realizedProfitCents)}</strong>
            <small>{percent(selected.simulation.realizedRoiBps)} pessimistic ROI</small>
          </div>
          <div>
            <span>Verification</span>
            <strong>{verified ? `${verified.testsPassed} / ${verified.testsPassed + verified.testsFailed}` : "—"}</strong>
            <small>{verified ? "workspace tests passed" : "not captured in this run"}</small>
          </div>
        </section>

        <section className="section-block" id="performance" aria-labelledby="performance-heading">
          <div className="section-intro">
            <h2 id="performance-heading">Latency stays sub-microsecond across the measured tail.</h2>
            <p>
              The chart keeps the 50 µs engineering budget in frame without flattening the measured distribution. Every point is service time inside the dedicated engine thread—not exchange transit or sportsbook polling latency.
            </p>
          </div>
          <Suspense fallback={<ChartFallback />}>
            <LatencyProfileChart selected={selected} runs={runs} />
          </Suspense>
          <div className="throughput-layout">
            <div className="comparison-copy">
              <strong>{throughputDelta(selected, baseline) >= 0 ? "+" : ""}{throughputDelta(selected, baseline).toFixed(1)}%</strong>
              <h3>Host-to-host throughput difference</h3>
              <p>
                {selected.environment.label} processed {compact(selected.performance.throughputPerSecond)} updates/sec across {selected.workload.feedEvents.toLocaleString()} generated feed events. Hardware differs, so this is comparison—not a regression series.
              </p>
            </div>
            <Suspense fallback={<ChartFallback />}>
              <ThroughputChart selected={selected} runs={runs} />
            </Suspense>
          </div>
        </section>

        <section className="section-block execution-section" id="execution" aria-labelledby="execution-heading">
          <div className="section-intro section-intro--split">
            <h2 id="execution-heading">A signal only counts after it survives the trip.</h2>
            <p>
              The simulator applies asymmetric wire delay, venue processing, queue front-running, depth loss, fees, and integer contract sizing. No clean fills were assumed in the published workload.
            </p>
          </div>

          <div className="execution-plot" role="img" aria-label={`${selected.simulation.totalSignals} signals: ${selected.simulation.proportionalFills} proportional fills, ${selected.simulation.cleanFills} clean fills, and ${selected.simulation.phantoms} phantom signals`}>
            <div className="execution-track">
              <span
                className="execution-fill"
                style={{ width: `${executionRate(selected)}%` }}
                title={`${selected.simulation.proportionalFills} executable with hedge preserved`}
              />
              <span
                className="execution-phantom"
                style={{ width: `${100 - executionRate(selected)}%` }}
                title={`${selected.simulation.phantoms} phantom signals`}
              />
            </div>
            <div className="execution-labels">
              <div><strong>{selected.simulation.proportionalFills}</strong><span>proportional fills</span></div>
              <div><strong>{executionRate(selected).toFixed(2)}%</strong><span>executable with hedge preserved</span></div>
              <div className="loss"><strong>{selected.simulation.phantoms}</strong><span>decayed in flight</span></div>
            </div>
          </div>

          <div className="financial-ledger" aria-label="Paper trading financial ledger">
            <div><span>Cumulative stake</span><strong>{money(selected.simulation.filledStakeCents)}</strong></div>
            <div><span>Venue fees paid</span><strong>{money(selected.simulation.feesPaidCents)}</strong></div>
            <div className="financial-ledger__result"><span>Realized worst-case profit</span><strong>{money(selected.simulation.realizedProfitCents)}</strong></div>
            <div><span>Settlement ROI</span><strong>{percent(selected.simulation.realizedRoiBps)}</strong></div>
          </div>

          <p className="method-note">
            These are deterministic paper-trading results from a synthetic market stream. They do not represent live trading, future returns, or an order-placement system.
          </p>
        </section>

        <section className="section-block trades-section" id="trades" aria-labelledby="trades-heading">
          <div className="section-intro">
            <h2 id="trades-heading">Every trade carries its own receipt.</h2>
            <p>
              Each detected signal is persisted beside its simulated outcome, so expected-vs-realized
              PnL is auditable at trade granularity instead of trusting aggregate counters. Numbers
              are the pipeline's own pessimistic integers, formatted but never recomputed.
            </p>
          </div>
          <Suspense fallback={<ChartFallback />}>
            {tradeStatus === "loading" ? (
              <div className="chart-fallback" role="status">Opening the trade log…</div>
            ) : (
              <TradeLedger log={tradeLog} error={tradeStatus === "error" ? tradeError : undefined} />
            )}
          </Suspense>
        </section>

        <section className="section-block verification-section" id="verification" aria-labelledby="verification-heading">
          <div className="verification-copy">
            <h2 id="verification-heading">Correctness is measured beside speed.</h2>
            {verified ? (
              <>
                <div className="verification-score">
                  <strong>{verified.testsPassed}</strong>
                  <span>passed</span>
                  <em>{verified.testsFailed} failed · {verified.clippyWarnings} warnings</em>
                </div>
                <p>
                  The suite spans fixed-point arithmetic, pessimistic payouts, parsers, sequence gaps, canonical matching, ring buffers, engine behavior, and simulated execution.
                </p>
              </>
            ) : (
              <p>This measured run did not capture the separate workspace verification snapshot.</p>
            )}
          </div>
          <Suspense fallback={<ChartFallback />}>
            <VerificationChart run={selected} />
          </Suspense>
        </section>

        <section className="section-block history-section" id="history" aria-labelledby="history-heading">
          <div className="section-intro">
            <h2 id="history-heading">Every published run keeps its context.</h2>
            <p>
              Dated snapshots are reviewed and committed rather than uploaded from an unaudited runtime. Runs from different operating systems or architectures remain visibly distinct.
            </p>
          </div>
          <div className="history-table-wrap">
            <table className="history-table">
              <thead>
                <tr>
                  <th scope="col">Run</th>
                  <th scope="col">Environment</th>
                  <th scope="col">Throughput</th>
                  <th scope="col">p99</th>
                  <th scope="col">Headroom</th>
                  <th scope="col">Source</th>
                </tr>
              </thead>
              <tbody>
                {runs.map((run) => (
                  <tr key={run.run.id} className={run.run.id === selected.run.id ? "is-selected" : undefined}>
                    <th scope="row">
                      <button type="button" onClick={() => setSelectedId(run.run.id)}>{formatDate(run)}</button>
                    </th>
                    <td>{run.environment.label}<small>{run.environment.arch}</small></td>
                    <td>{compact(run.performance.throughputPerSecond)}/s</td>
                    <td>{nsToMicros(run.performance.latencyNs.p99).toFixed(3)} µs</td>
                    <td>{headroom(run).toFixed(0)}×</td>
                    <td>{run.run.source === "published-snapshot" ? "Published report" : "Measured JSON"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>

          <details className="methodology">
            <summary>Inspect selected-run provenance and methodology</summary>
            <dl>
              <div><dt>Environment</dt><dd>{selected.environment.os} · {selected.environment.arch}</dd></div>
              <div><dt>Compiler</dt><dd>{selected.environment.rustc ?? "Not recorded"}</dd></div>
              <div><dt>Build</dt><dd>{selected.environment.buildProfile}</dd></div>
              <div><dt>Workload</dt><dd>{selected.workload.feedEvents.toLocaleString()} feed events · {selected.workload.market}</dd></div>
              <div><dt>Venues</dt><dd>{selected.workload.venues.join(", ")}</dd></div>
              <div><dt>Commit</dt><dd>{selected.run.gitCommit ?? "Not recorded in imported report"}</dd></div>
            </dl>
          </details>

          <div className="history-actions">
            <button type="button" onClick={downloadSnapshot}>Download selected JSON</button>
            <a href="https://github.com/harlanljones/arbkit/blob/main/RESULTS.md">Read the execution report</a>
            <a href="https://github.com/harlanljones/arbkit">Inspect the source</a>
          </div>
        </section>
      </main>

      <footer>
        <strong>arbkit</strong>
        <p>Cross-venue detection with pessimistic arithmetic. Live order placement remains out of scope.</p>
        <a href="#top">Back to the p99 result</a>
      </footer>
    </div>
  );
}
