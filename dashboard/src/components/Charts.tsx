import {
  Bar,
  BarChart,
  CartesianGrid,
  Cell,
  LabelList,
  Legend,
  Line,
  LineChart,
  ReferenceLine,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import type { RunSnapshot } from "../data/schema";
import { compact, nsToMicros } from "../data/metrics";

const INK = "#171814";
const GREEN = "#2f6d43";
const COPPER = "#9a4d2f";
const BLUE = "#315a78";
const PAPER = "#f3efe5";

type RunsProps = {
  selected: RunSnapshot;
  runs: RunSnapshot[];
};

const tooltipStyle = {
  background: PAPER,
  border: `1px solid ${INK}`,
  borderRadius: 0,
  fontFamily: "IBM Plex Mono, monospace",
  fontSize: 12,
};

export function LatencyProfileChart({ selected, runs }: RunsProps) {
  const keys = ["p50", "p90", "p99", "p999", "max"] as const;
  const data = keys.map((key) => ({
    percentile: key === "p999" ? "p99.9" : key,
    selected: selected.performance.latencyNs[key],
    comparison:
      runs.find((run) => run.run.id !== selected.run.id)?.performance.latencyNs[key] ?? null,
  }));
  const comparison = runs.find((run) => run.run.id !== selected.run.id);

  return (
    <figure className="chart-figure">
      <div className="chart-frame chart-frame--latency">
        <ResponsiveContainer width="100%" height="100%">
          <LineChart data={data} margin={{ top: 22, right: 28, bottom: 10, left: 4 }} accessibilityLayer>
            <CartesianGrid stroke="#d7d0c0" strokeDasharray="2 5" vertical={false} />
            <XAxis dataKey="percentile" tickLine={false} axisLine={{ stroke: INK }} />
            <YAxis
              scale="log"
              domain={[10, 100_000]}
              allowDataOverflow
              tickFormatter={(value) => `${nsToMicros(value)}µs`}
              ticks={[10, 100, 1_000, 10_000, 100_000]}
              tickLine={false}
              axisLine={false}
              width={60}
            />
            <Tooltip
              contentStyle={tooltipStyle}
              formatter={(value) => [`${nsToMicros(Number(value)).toFixed(3)} µs`]}
            />
            <Legend iconType="square" iconSize={9} verticalAlign="top" align="right" height={34} />
            <ReferenceLine
              y={selected.performance.targetP99Ns}
              stroke={COPPER}
              strokeDasharray="6 5"
              label={{ value: "50 µs budget", fill: COPPER, position: "insideTopLeft" }}
            />
            <Line
              name={selected.environment.label}
              dataKey="selected"
              stroke={GREEN}
              strokeWidth={3}
              dot={{ fill: GREEN, r: 4, strokeWidth: 0 }}
              activeDot={{ r: 6 }}
            />
            {comparison && (
              <Line
                name={comparison.environment.label}
                dataKey="comparison"
                stroke={BLUE}
                strokeWidth={2}
                strokeDasharray="5 5"
                dot={{ fill: BLUE, r: 3, strokeWidth: 0 }}
              />
            )}
          </LineChart>
        </ResponsiveContainer>
      </div>
      <figcaption>
        Service-time percentiles use a logarithmic axis so the measured distribution and distant budget remain honest and legible.
      </figcaption>
      <details className="data-table">
        <summary>View latency data table</summary>
        <table>
          <thead>
            <tr>
              <th scope="col">Percentile</th>
              <th scope="col">{selected.environment.label}</th>
              {comparison && <th scope="col">{comparison.environment.label}</th>}
            </tr>
          </thead>
          <tbody>
            {data.map((row) => (
              <tr key={row.percentile}>
                <th scope="row">{row.percentile}</th>
                <td>{nsToMicros(row.selected).toFixed(3)} µs</td>
                {comparison && <td>{nsToMicros(row.comparison ?? 0).toFixed(3)} µs</td>}
              </tr>
            ))}
          </tbody>
        </table>
      </details>
    </figure>
  );
}

export function ThroughputChart({ selected, runs }: RunsProps) {
  const data = runs.map((run) => ({
    id: run.run.id,
    name: run.environment.label,
    value: run.performance.throughputPerSecond,
    selected: run.run.id === selected.run.id,
  }));

  return (
    <figure className="chart-figure">
      <div className="chart-frame chart-frame--throughput">
        <ResponsiveContainer width="100%" height="100%">
          <BarChart data={data} layout="vertical" margin={{ top: 6, right: 70, bottom: 6, left: 26 }} accessibilityLayer>
            <CartesianGrid stroke="#d7d0c0" strokeDasharray="2 5" horizontal={false} />
            <XAxis type="number" tickFormatter={compact} axisLine={{ stroke: INK }} tickLine={false} />
            <YAxis dataKey="name" type="category" width={132} tickLine={false} axisLine={false} />
            <Tooltip contentStyle={tooltipStyle} formatter={(value) => [`${Number(value).toLocaleString()} updates/sec`]} />
            <Bar dataKey="value" name="Updates per second" radius={0}>
              {data.map((entry) => (
                <Cell key={entry.id} fill={entry.selected ? GREEN : BLUE} />
              ))}
              <LabelList dataKey="value" position="right" formatter={(value) => compact(Number(value))} />
            </Bar>
          </BarChart>
        </ResponsiveContainer>
      </div>
      <figcaption>Host-to-host comparison, not a same-machine regression claim. Higher is better.</figcaption>
      <details className="data-table">
        <summary>View throughput data table</summary>
        <table>
          <thead>
            <tr>
              <th scope="col">Environment</th>
              <th scope="col">Updates per second</th>
              <th scope="col">Selected</th>
            </tr>
          </thead>
          <tbody>
            {data.map((row) => (
              <tr key={row.id}>
                <th scope="row">{row.name}</th>
                <td>{Number(row.value).toLocaleString()}</td>
                <td>{row.selected ? "Selected" : ""}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </details>
    </figure>
  );
}

export function VerificationChart({ run }: { run: RunSnapshot }) {
  const data = run.verification?.crates ?? [];
  if (!run.verification) {
    return <p className="empty-inline">This measured run does not include a test-suite snapshot.</p>;
  }
  return (
    <figure className="chart-figure">
      <div className="chart-frame chart-frame--tests">
        <ResponsiveContainer width="100%" height="100%">
          <BarChart data={data} margin={{ top: 26, right: 16, bottom: 4, left: 0 }} accessibilityLayer>
            <CartesianGrid stroke="#d7d0c0" strokeDasharray="2 5" vertical={false} />
            <XAxis dataKey="name" tickLine={false} axisLine={{ stroke: INK }} />
            <YAxis allowDecimals={false} tickLine={false} axisLine={false} width={30} />
            <Tooltip contentStyle={tooltipStyle} formatter={(value) => [`${value} tests`]} />
            <Bar dataKey="tests" fill={GREEN} radius={0}>
              <LabelList dataKey="tests" position="top" />
            </Bar>
          </BarChart>
        </ResponsiveContainer>
      </div>
      <figcaption>Unit, property, integration, and documentation tests grouped by workspace crate.</figcaption>
      <details className="data-table">
        <summary>View verification data table</summary>
        <table>
          <thead>
            <tr>
              <th scope="col">Workspace crate</th>
              <th scope="col">Tests passed</th>
            </tr>
          </thead>
          <tbody>
            {data.map((crate) => (
              <tr key={crate.name}>
                <th scope="row">arbkit-{crate.name}</th>
                <td>{crate.tests}</td>
              </tr>
            ))}
          </tbody>
          <tfoot>
            <tr>
              <th scope="row">Total</th>
              <td>{run.verification.testsPassed}</td>
            </tr>
          </tfoot>
        </table>
      </details>
    </figure>
  );
}
