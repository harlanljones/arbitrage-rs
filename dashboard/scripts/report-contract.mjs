export function validateReport(run) {
  if (
    run?.schemaVersion !== 1 ||
    typeof run.run?.id !== "string" ||
    typeof run.run?.recordedAtEpochMs !== "number" ||
    run.run?.source !== "measured" ||
    typeof run.environment?.os !== "string" ||
    typeof run.environment?.arch !== "string" ||
    run.workload?.synthetic !== true ||
    run.workload?.paperTrading !== true ||
    typeof run.performance?.throughputPerSecond !== "number" ||
    typeof run.performance?.latencyNs?.p99 !== "number" ||
    typeof run.performance?.targetP99Ns !== "number" ||
    typeof run.simulation?.realizedProfitCents !== "number"
  ) {
    throw new Error("Pipeline output did not match benchmark snapshot schema version 1.");
  }
  return run;
}
