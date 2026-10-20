// Reads one `jd --json diag bench` envelope from stdin, prints the per-stage numbers and fails
// above the smoke budget. Usage: jd --json diag bench s1 | tail -1 | node scripts/check-bench.mjs [maxMs] [maxBytes]
import { readFileSync } from "node:fs";

const maxMs = Number(process.argv[2] ?? 15000);
const maxBytes = Number(process.argv[3] ?? 1024 ** 3);
const baselinePath = process.argv[4]; // optional: scripts/bench-baseline.json (per-OS totals + the expected content hash)
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
if (baselinePath) {
  const b = JSON.parse(readFileSync(baselinePath, "utf8"));
  if (!r.content_hash.startsWith(b.content_hash)) {
    console.error(`::error::S1 content hash ${r.content_hash.slice(0, 16)} differs from the baseline ${b.content_hash}: the kernels no longer produce the same terrain (update scripts/bench-baseline.json only for an intended change)`);
    process.exit(1);
  }
  const base = b.total_ms[process.platform];
  if (base) {
    const ratio = r.total_ms / base;
    console.log(`S1 vs baseline (${process.platform} ${base} ms): ${ratio.toFixed(2)}×`);
    if (ratio > 3) {
      console.error(`::error::S1 is ${ratio.toFixed(1)}× the ${process.platform} baseline (${base} ms)`);
      process.exit(1);
    }
    if (ratio > 1.5) console.log(`::warning::S1 is ${ratio.toFixed(2)}× the ${process.platform} baseline (${base} ms); runners vary, but check the kernels if this persists`);
  }
}
