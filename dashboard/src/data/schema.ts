import { z } from "zod";

const nonNegative = z.number().nonnegative();
const whole = nonNegative.int();

export const RunIndexSchema = z.object({
  schemaVersion: z.literal(1),
  runs: z.array(
    z.object({
      id: z.string().min(1),
      file: z.string().min(1),
    }),
  ),
});

export const RunSnapshotSchema = z.object({
  schemaVersion: z.literal(1),
  run: z.object({
    id: z.string().min(1),
    recordedAt: z.string().min(1).optional(),
    recordedAtEpochMs: nonNegative.optional(),
    source: z.enum(["measured", "published-snapshot"]),
    projectVersion: z.string().min(1),
    gitCommit: z.string().nullable().optional(),
  }),
  environment: z.object({
    label: z.string().min(1),
    os: z.string().min(1),
    arch: z.string().min(1),
    rustc: z.string().nullable().optional(),
    buildProfile: z.string().min(1),
  }),
  workload: z.object({
    synthetic: z.boolean(),
    paperTrading: z.boolean(),
    feedEvents: whole,
    event: z.string().min(1),
    market: z.string().min(1),
    venues: z.array(z.string().min(1)).min(1),
  }),
  performance: z.object({
    elapsedMs: nonNegative,
    throughputPerSecond: nonNegative,
    targetP99Ns: nonNegative.positive(),
    latencyNs: z.object({
      count: whole.optional(),
      min: whole,
      mean: whole,
      p50: whole,
      p90: whole,
      p99: whole,
      p999: whole,
      max: whole,
    }),
  }),
  detection: z.object({
    eventsProcessed: whole,
    signalsEmitted: whole,
    collectedSignals: whole,
    sample: z
      .object({
        profitBps: whole,
        totalStakeCents: z.number().int(),
        guaranteedProfitCents: z.number().int(),
      })
      .nullable()
      .optional(),
  }),
  simulation: z.object({
    totalSignals: whole,
    cleanFills: whole,
    proportionalFills: whole,
    phantoms: whole,
    phantomRateBps: whole,
    filledStakeCents: z.number().int(),
    feesPaidCents: z.number().int(),
    realizedProfitCents: z.number().int(),
    realizedRoiBps: z.number().int(),
  }),
  verification: z
    .object({
      testsPassed: whole,
      testsFailed: whole,
      clippyWarnings: whole,
      crates: z.array(
        z.object({
          name: z.string().min(1),
          tests: whole,
        }),
      ),
    })
    .optional(),
});

export type RunIndex = z.infer<typeof RunIndexSchema>;
export type RunSnapshot = z.infer<typeof RunSnapshotSchema>;

export async function loadRunHistory(): Promise<RunSnapshot[]> {
  const dataRoot = `${import.meta.env.BASE_URL}data/runs/`;
  const indexResponse = await fetch(`${dataRoot}index.json`);
  if (!indexResponse.ok) {
    throw new Error(`The run index returned ${indexResponse.status}.`);
  }
  const index = RunIndexSchema.parse(await indexResponse.json());
  const runs = await Promise.all(
    index.runs.map(async ({ file }) => {
      const response = await fetch(`${dataRoot}${file}`);
      if (!response.ok) {
        throw new Error(`${file} returned ${response.status}.`);
      }
      return RunSnapshotSchema.parse(await response.json());
    }),
  );
  return runs.sort((a, b) => timestampOf(b) - timestampOf(a));
}

export function timestampOf(run: RunSnapshot): number {
  if (run.run.recordedAtEpochMs !== undefined) return run.run.recordedAtEpochMs;
  if (run.run.recordedAt) return new Date(`${run.run.recordedAt}T12:00:00Z`).getTime();
  return 0;
}
