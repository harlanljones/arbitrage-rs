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
const indexPath = join(runsDir, "index.json");

const result = spawnSync(
  "cargo",
  ["run", "--example", "pipeline", "--release", "--", "--json", pendingPath],
  { cwd: repoDir, stdio: "inherit" },
);

if (result.error) throw result.error;
if (result.status !== 0) process.exit(result.status ?? 1);

try {
  const run = validateReport(JSON.parse(readFileSync(pendingPath, "utf8")));

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

  run.run.id = id;
  writeFileSync(pendingPath, `${JSON.stringify(run, null, 2)}\n`);
  renameSync(pendingPath, finalPath);

  const index = JSON.parse(readFileSync(indexPath, "utf8"));
  if (index.schemaVersion !== 1 || !Array.isArray(index.runs)) {
    throw new Error("Run index did not match schema version 1.");
  }
  index.runs = [{ id, file: filename }, ...index.runs];
  const pendingIndex = `${indexPath}.tmp`;
  writeFileSync(pendingIndex, `${JSON.stringify(index, null, 2)}\n`);
  renameSync(pendingIndex, indexPath);
  console.log(`Published benchmark snapshot ${filename}`);
} finally {
  if (existsSync(pendingPath)) rmSync(pendingPath);
}
