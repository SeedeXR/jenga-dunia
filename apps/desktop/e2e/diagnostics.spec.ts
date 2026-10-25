import { expect, test } from "@playwright/test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, evaluateAndWait, installBridge } from "./bridge";

test.describe("tasks and diagnostics panel against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-diag-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true });
  });

  test("environment, cache GC and a small bench run from the panel and show their numbers", async ({ page }, testInfo) => {
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto("/");
    await expect(page.getByTestId("tasks")).toContainText("no running jobs");
    await page.getByTestId("diag-env").click();
    await expect(page.getByTestId("log")).toContainText("diag.env: ok");
    await expect(page.getByTestId("diag-results")).toContainText(/\d+ cpus · \d+ GB/);

    await page.getByTestId("project-path").fill(path.join(ws, "Diag.jdproj"));
    await page.getByLabel("Extent (m)").fill("1024");
    await page.getByLabel("Resolution (px)").fill("128");
    await page.getByTestId("new-project").click();
    await expect(page.locator("header")).toContainText("World — 128²");
    await evaluateAndWait(page, "generate");
    await expect(page.getByTestId("layer-stats")).toBeVisible();
    // One layer in the cache; GC keeps it and reports the project's limit.
    await page.getByTestId("diag-gc").click();
    await expect(page.getByTestId("log")).toContainText("project.cache_gc: ok");
    await expect(page.getByTestId("diag-results")).toContainText(/1 blobs · 0\.1 MB · evicted 0 · limit 2048 MB/);

    await page.getByTestId("diag-bench").click();
    await expect(page.getByTestId("log")).toContainText("diag.bench: ok", { timeout: 60_000 });
    await expect(page.getByTestId("diag-results")).toContainText(/S1 256² (debug|release): total \d+ ms/);
    await expect(page.getByTestId("diag-results")).toContainText("thermal×50");
    await page.screenshot({ path: testInfo.outputPath("diagnostics.png") }); // tutorial image source
    expect(errors, "no uncaught page errors").toEqual([]);
    await core.close();
  });
});
