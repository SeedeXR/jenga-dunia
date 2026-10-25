import { expect, test } from "@playwright/test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, installBridge } from "./bridge";

test.describe("jobs against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-jobs-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true });
  });

  test("a long evaluation shows progress in the header and can be cancelled", async ({ page }) => {
    const core = await connectCore(ws);
    await installBridge(page, core);
    await page.goto("/");
    await page.getByTestId("project-path").fill(path.join(ws, "Jobs.jdproj"));
    await page.getByLabel("Extent (m)").fill("1024");
    await page.getByLabel("Resolution (px)").fill("256");
    await page.getByTestId("new-project").click();
    await expect(page.getByTestId("viewport-info")).toContainText(/ [1-9][\d,]* tris/);
    // Turn live preview off, then wire a very long hydraulic run in front of the output.
    await page.getByTestId("graph-live").click();
    await page.getByTestId("graph-kind").selectOption("jd.erosion.hydraulic@1");
    await page.getByTestId("graph-add").click();
    const panel = page.getByTestId("graph-panel");
    const noise = panel.locator(".react-flow__node", { hasText: "jd.noise.fbm@1" });
    const hyd = panel.locator(".react-flow__node", { hasText: "jd.erosion.hydraulic@1" });
    const out = panel.locator(".react-flow__node", { hasText: "jd.output.height@1" });
    await hyd.click();
    const iterations = page.getByTestId("graph-inspector").locator("label", { hasText: "iterations" }).locator("input");
    await iterations.fill("2000"); // the node's maximum; with a 256² world this runs for seconds
    await iterations.blur();
    await expect(page.getByTestId("log")).toContainText("graph.set_param: ok");
    const drag = async (from: ReturnType<typeof page.locator>, to: ReturnType<typeof page.locator>) => {
      await to.hover();
      await from.hover();
      const a = (await from.boundingBox())!;
      const b = (await to.boundingBox())!;
      await page.mouse.move(a.x + a.width / 2, a.y + a.height / 2);
      await page.mouse.down();
      await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2, { steps: 10 });
      await page.mouse.up();
    };
    await drag(noise.locator(".react-flow__handle.source"), hyd.locator('.react-flow__handle.target[data-handleid="in"]')); // the node also has a `hardness` input
    await drag(hyd.locator(".react-flow__handle.source[data-handleid=\"out\"]"), out.locator(".react-flow__handle.target"));
    await expect(panel.locator(".react-flow__edge")).toHaveCount(2);

    // Evaluate: the header shows the running job; Cancel stops it with E_CANCELLED.
    await page.getByTestId("graph-evaluate").click();
    await expect(page.getByTestId("job-progress")).toContainText("graph.evaluate", { timeout: 15_000 });
    await page.getByTestId("job-cancel").click();
    await expect(page.getByTestId("log")).toContainText("E_CANCELLED", { timeout: 30_000 });
    await expect(page.getByTestId("job-progress")).toHaveCount(0);

    // Esc aborts too (outside text fields): a second evaluation, then the key.
    const cancelledBefore = await page.getByTestId("log").locator("div", { hasText: "E_CANCELLED" }).count();
    await page.getByTestId("graph-evaluate").click();
    await expect(page.getByTestId("job-progress")).toContainText("Esc to abort", { timeout: 15_000 });
    await page.locator("body").press("Escape");
    await expect.poll(() => page.getByTestId("log").locator("div", { hasText: "E_CANCELLED" }).count(), { timeout: 30_000 }).toBeGreaterThan(cancelledBefore);
    await expect(page.getByTestId("job-progress")).toHaveCount(0);
    await core.close();
  });
});
