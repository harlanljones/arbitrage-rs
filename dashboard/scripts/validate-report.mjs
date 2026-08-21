import { readFileSync } from "node:fs";
import { validateReport } from "./report-contract.mjs";

const path = process.argv[2];
if (!path) {
  console.error("Usage: npm run validate:report -- <snapshot.json>");
  process.exit(2);
}

validateReport(JSON.parse(readFileSync(path, "utf8")));
console.log(`Validated benchmark snapshot ${path}`);
