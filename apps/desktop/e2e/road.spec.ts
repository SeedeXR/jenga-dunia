import { expect, test } from "@playwright/test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, installBridge, logCount } from "./bridge";

/** Draw a road: one viewport drag becomes a spline node that flattens a road bed along it. */
test.describe("roads against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-road-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true });
  });

  test("Road arms the next drag; the drag splices a spline node before the output and the terrain changes", async ({ page }, testInfo) => {
    test.slow();
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto("/");
    await page.getByTestId("project-path").fill(path.join(ws, "Road.jdproj"));
    await page.getByLabel("Extent (m)").fill("1280");
    await page.getByLabel("Resolution (px)").fill("128");
    await page.getByTestId("new-project").click();
    await expect(page.locator("header")).toContainText("World — 128²");
    await expect(page.getByTestId("layer-stats")).toBeVisible();
    const stats = page.getByTestId("layer-stats");
    const hashBefore = (await stats.textContent())?.match(/hash([0-9a-f]+)/)?.[1];
    expect(hashBefore).toBeTruthy();

    await page.getByTestId("road-draw").click();
    await expect(page.getByTestId("road-draw")).toContainText("Drawing road");
    const vp = (await page.getByTestId("viewport").boundingBox())!;
    const [cx, cy] = [vp.x + vp.width / 2, vp.y + vp.height / 2];
    await page.mouse.move(cx - 120, cy + 20);
    await page.mouse.down();
    await page.mouse.move(cx + 120, cy - 30, { steps: 16 });
    await page.mouse.up();
    // One road node, spliced in and selected; the button is back to orbiting.
    const panel = page.getByTestId("graph-panel");
    const road = panel.locator(".react-flow__node", { hasText: "jd.terrain.spline@1" });
    await expect(road).toHaveCount(1);
    await expect(road).toContainText("width_m=12");
    await expect(page.getByTestId("graph-inspector")).toContainText("jd.terrain.spline@1");
    await expect(page.getByTestId("road-draw")).toContainText("Road");
    // One splice in front of the height (the core's graph.splice adds and wires the node).
    await expect(page.getByTestId("log")).toContainText("graph.splice: ok");
    await expect(page.getByTestId("log").locator("div", { hasText: "graph.splice: ok" })).toHaveCount(1);
    // The live preview re-evaluates: the published height changes under the road.
    await expect.poll(async () => (await stats.textContent())?.match(/hash([0-9a-f]+)/)?.[1], { timeout: 30_000 }).not.toBe(hashBefore);
    await page.screenshot({ path: testInfo.outputPath("road.png") }); // tutorial image source
    expect(errors).toEqual([]);
  });
});
