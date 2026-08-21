import { nsToMicros } from "../data/metrics";
import type { RunSnapshot } from "../data/schema";

export function BudgetRuler({ selected }: { selected: RunSnapshot }) {
  const p99 = selected.performance.latencyNs.p99;
  const budget = selected.performance.targetP99Ns;
  const min = 1;
  const max = 10_000_000;
  const y = (value: number) => 438 - (Math.log10(value / min) / Math.log10(max / min)) * 372;
  const ticks = [1, 10, 100, 1_000, 10_000, 100_000, 1_000_000, 10_000_000];

  return (
    <figure className="budget-ruler">
      <svg viewBox="0 0 300 500" role="img" aria-labelledby="budget-ruler-title budget-ruler-desc">
        <title id="budget-ruler-title">P99 hot-loop latency against its budget</title>
        <desc id="budget-ruler-desc">
          {p99} nanoseconds measured compared with a {budget} nanosecond budget on a logarithmic scale.
        </desc>
        <line className="ruler-spine" x1="148" x2="148" y1="56" y2="448" />
        {ticks.map((tick) => (
          <g key={tick}>
            <line className="ruler-tick" x1="137" x2="159" y1={y(tick)} y2={y(tick)} />
            <text className="ruler-label" x="169" y={y(tick) + 4}>
              {tick < 1_000 ? `${tick} ns` : `${tick / 1_000} µs`}
            </text>
          </g>
        ))}
        <line className="ruler-budget" x1="50" x2="250" y1={y(budget)} y2={y(budget)} />
        <circle className="ruler-budget-dot" cx="148" cy={y(budget)} r="6" />
        <text className="ruler-budget-text" x="48" y={y(budget) - 12}>50 µs budget</text>
        <line className="ruler-measure" x1="70" x2="230" y1={y(p99)} y2={y(p99)} />
        <circle className="ruler-measure-dot" cx="148" cy={y(p99)} r="7" />
        <text className="ruler-measure-text" x="48" y={y(p99) + 27}>
          {nsToMicros(p99).toFixed(3)} µs p99
        </text>
      </svg>
      <figcaption>Logarithmic scale · nanoseconds to milliseconds</figcaption>
    </figure>
  );
}
