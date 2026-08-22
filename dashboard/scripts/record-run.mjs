import { spawnSync } from "node:child_process";
import { existsSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { validateReport } from "./report-contract.mjs";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const dashboardDir = resolve(scriptDir, "..");
const repoDir = resolve(dashboardDir, "..");
const runsDir = join(dashboardDir, "public", "data", "runs");
const pendingPath = join(runsDir, `.pending-${process.pid}.json`);
const pendingTradesPath = join(runsDir, `.pending-${process.pid}.trades.jsonl`);
const indexPath = join(runsDir, "index.json");

const result = spawnSync(
  "cargo",
  [
    "run", "--example", "pipeline", "--release", "--",
    "--json", pendingPath,
    "--trades", pendingTradesPath,
  ],
  { cwd: repoDir, stdio: "inherit" },
);

if (result.error) throw result.error;
if (result.status !== 0) process.exit(result.status ?? 1);

// Validates a pipeline-produced trades JSONL before anything is published:
// header shape, header count vs record lines, and runId agreement. A ledger
// that fails here is proof of nothing, so recording fails loudly instead.
function readPendingTrades(expectedRunId) {
  if (!existsSync(pendingTradesPath)) return null;
  const lines = readFileSync(pendingTradesPath, "utf8")
    .split("\n")
    .filter((line) => line.trim().length > 0);
  const header = JSON.parse(lines[0]);
  if (header.schemaVersion !== 1 || header.kind !== "arbkit-trades") {
    throw new Error("Trade log header did not match schema version 1 / kind arbkit-trades.");
  }
  if (header.runId !== expectedRunId) {
    throw new Error(
      `Trade log header claims run ${header.runId}, expected ${expectedRunId}.`,
    );
  }
  if (header.tradeCount !== lines.length - 1) {
    throw new Error(
      `Trade log header counts ${header.tradeCount} trades but ${lines.length - 1} lines follow.`,
    );
  }
  return { header, lines };
}

try {
  const run = validateReport(JSON.parse(readFileSync(pendingPath, "utf8")));
  const pipelineRunId = run.run.id;
  const trades = readPendingTrades(pipelineRunId);

  run.run.recordedAt = new Date(run.run.recordedAtEpochMs).toISOString();
  const stamp = run.run.recordedAt.replaceAll(":", "").replaceAll(".", "-");
  const commit = run.run.gitCommit ?? "working-tree";
  const safe = (value) => String(value).toLowerCase().replaceAll(/[^a-z0-9_-]+/g, "-");
  const id = `${stamp}-${safe(run.environment.os)}-${safe(run.environment.arch)}-${safe(commit)}`;
  const filename = `${id}.json`;
  const finalPath = join(runsDir, filename);

  if (existsSync(finalPath)) {
    throw new Error(`Refusing to overwrite existing run ${filename}.`);
  }

  // The recorder renames the run, so the trade header must be renamed with
  // it to keep the frozen `runId`-matches-`run.id` invariant intact.
  let tradesFilename;
  if (trades) {
    tradesFilename = `${id}.trades.jsonl`;
    const renamedHeader = { ...trades.header, runId: id };
    writeFileSync(
      join(runsDir, tradesFilename),
      [JSON.stringify(renamedHeader), ...trades.lines.slice(1)].join("\n") + "\n",
    );
    rmSync(pendingTradesPath);
  }

  run.run.id = id;
  writeFileSync(pendingPath, `${JSON.stringify(run, null, 2)}\n`);
  renameSync(pendingPath, finalPath);

  const index = JSON.parse(readFileSync(indexPath, "utf8"));
  if (index.schemaVersion !== 1 || !Array.isArray(index.runs)) {
    throw new Error("Run index did not match schema version 1.");
  }
  index.runs = [{ id, file: filename, ...(trades && { tradesFile: tradesFilename }) }, ...index.runs];
  const pendingIndex = `${indexPath}.tmp`;
  writeFileSync(pendingIndex, `${JSON.stringify(index, null, 2)}\n`);
  renameSync(pendingIndex, indexPath);
  console.log(`Published benchmark snapshot ${filename}${trades ? ` with ${tradesFilename}` : ""}`);
} finally {
  if (existsSync(pendingPath)) rmSync(pendingPath);
  if (existsSync(pendingTradesPath)) rmSync(pendingTradesPath);
}
