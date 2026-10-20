// Reads one `jd --json diag bench` envelope from stdin, prints the per-stage numbers and fails
// above the smoke budget. Usage: jd --json diag bench s1 | tail -1 | node scripts/check-bench.mjs [maxMs] [maxBytes]
import { readFileSync } from "node:fs";

const maxMs = Number(process.argv[2] ?? 15000);
const maxBytes = Number(process.argv[3] ?? 1024 ** 3);
const d = JSON.parse(readFileSync(0, "utf8"));
if (d.status !== "ok") {
  console.error("bench failed:", JSON.stringify(d.error ?? d));
  process.exit(1);
}
const r = d.result;
const stages = r.stages.map((s) => `${s.name} ${s.ms.toFixed(0)} ms`).join(", ");
const rss = r.peak_rss_bytes == null ? "n/a" : `${(r.peak_rss_bytes / 1048576).toFixed(0)} MB`;
console.log(`S1 ${r.size_px}² ${r.profile}: ${stages}; total ${r.total_ms.toFixed(0)} ms; peak RSS ${rss}; hash ${r.content_hash.slice(0, 16)}`);
if (r.total_ms >= maxMs) {
  console.error(`::error::S1 took ${r.total_ms.toFixed(0)} ms, over the ${maxMs} ms smoke budget`);
  process.exit(1);
}
if ((r.peak_rss_bytes ?? 0) >= maxBytes) {
  console.error(`::error::S1 peak RSS ${r.peak_rss_bytes} B is over the ${maxBytes} B budget`);
  process.exit(1);
}
