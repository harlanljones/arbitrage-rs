import type { RunSnapshot } from "./schema";

export const nsToMicros = (nanoseconds: number) => nanoseconds / 1_000;

export const headroom = (run: RunSnapshot) =>
  run.performance.targetP99Ns / run.performance.latencyNs.p99;

export const throughputDelta = (current: RunSnapshot, baseline: RunSnapshot) =>
  ((current.performance.throughputPerSecond - baseline.performance.throughputPerSecond) /
    baseline.performance.throughputPerSecond) *
  100;

export const executionRate = (run: RunSnapshot) => {
  if (run.simulation.totalSignals === 0) return 0;
  return (
    ((run.simulation.cleanFills + run.simulation.proportionalFills) /
      run.simulation.totalSignals) *
    100
  );
};

export const money = (cents: number) =>
  new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  }).format(cents / 100);

export const compact = (value: number) =>
  new Intl.NumberFormat("en-US", {
    notation: "compact",
    maximumFractionDigits: 2,
  }).format(value);

export const percent = (basisPoints: number) => `${(basisPoints / 100).toFixed(2)}%`;

export const formatDate = (run: RunSnapshot) => {
  const date = run.run.recordedAtEpochMs
    ? new Date(run.run.recordedAtEpochMs)
    : new Date(`${run.run.recordedAt}T12:00:00Z`);
  return new Intl.DateTimeFormat("en-US", {
    year: "numeric",
    month: "short",
    day: "numeric",
    timeZone: "UTC",
  }).format(date);
};
