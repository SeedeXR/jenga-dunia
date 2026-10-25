import { expect, test } from "@playwright/test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, evaluateAndWait, installBridge } from "./bridge";

// Measurement, not a gate: JD_MEASURE=1 runs it and prints the header's preview IPC numbers for
// docs/spec/performance-targets.md (u16 preview bytes over the MCP bridge → viewport, with decode).
test.describe("preview IPC measurement", () => {
  test.skip(!process.env.JD_MEASURE, "set JD_MEASURE=1 to measure");
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-ipc-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true });
  });

  for (const res of [512, 1024, 2048]) {
    test(`preview of a ${res}² world`, async ({ page }) => {
      test.slow();
      const core = await connectCore(ws);
      await installBridge(page, core);
      await page.goto("/");
      await page.getByTestId("project-path").fill(path.join(ws, `W${res}.jdproj`));
      await page.getByLabel("Extent (m)").fill(String(res * 4));
      await page.getByLabel("Resolution (px)").fill(String(res));
      await page.getByTestId("new-project").click();
      await expect(page.locator("header")).toContainText(`World — ${res}²`);
      await evaluateAndWait(page, "generate");
      await expect(page.getByTestId("layer-stats")).toBeVisible({ timeout: 120_000 });
      const info = page.getByTestId("viewport-info");
      await expect(info).toContainText(/preview lod \d+, [\d.]+ MB in \d+ ms/, { timeout: 120_000 });
      const text = (await info.textContent()) ?? "";
      const m = text.match(/preview lod (\d+), ([\d.]+) MB in (\d+) ms/);
      expect(m).toBeTruthy();
      const [, lod, mb, ms] = m!;
      console.log(`JD_IPC ${res}² lod ${lod}: ${mb} MB in ${ms} ms = ${((Number(mb) / Number(ms)) * 1000).toFixed(0)} MB/s (backend ${/webgpu/.test(text) ? "webgpu" : "webgl2"})`);
      await core.close();
    });
  }
});
