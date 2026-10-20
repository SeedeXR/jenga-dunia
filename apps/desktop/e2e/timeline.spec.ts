import { expect, test } from "@playwright/test";
import { mkdtempSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, fitView, installBridge, logCount, selectNode, wire } from "./bridge";

const NOISE = "01JDGRAPH000000000000N01SE"; // starter graph ids (jd_graph::nodes)
const HEIGHT = "01JDGRAPH0000000000HE1GHT0";

/** Keyframed parameters and the timeline slider against the real core. */
test.describe("timeline against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-time-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true });
  });

  test("thermal steps keyed 1 → 300: scrubbing the slider re-evaluates the terrain", async ({ page }, testInfo) => {
    test.slow();
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto("/");
    await page.getByTestId("project-path").fill(path.join(ws, "Time.jdproj"));
    await page.getByLabel("Extent (m)").fill("1280");
    await page.getByLabel("Resolution (px)").fill("128");
    await page.getByTestId("new-project").click();
    await expect(page.locator("header")).toContainText("World — 128²");
    await expect(page.getByTestId("graph-time")).toBeEnabled();
    await expect(page.getByTestId("graph-time-value")).toHaveText("0.00");

    const panel = page.getByTestId("graph-panel");
    const noise = panel.locator(`.react-flow__node[data-id="${NOISE}"]`);
    const height = panel.locator(`.react-flow__node[data-id="${HEIGHT}"]`);
    const inspector = page.getByTestId("graph-inspector");
    // A thermal node in front of the output, its step count keyed over the timeline.
    await page.getByTestId("graph-kind").selectOption("jd.erosion.thermal@1");
    await page.getByTestId("graph-add").click();
    const thermal = panel.locator(".react-flow__node", { hasText: "jd.erosion.thermal@1" });
    await fitView(page, thermal);
    await selectNode(thermal);
    const steps = inspector.locator("label", { hasText: "iterations" }).locator("input");
    await steps.fill("0:1, 1:300");
    await steps.blur();
    await expect(page.getByTestId("log")).toContainText("graph.set_param: ok");
    await expect(thermal).toContainText("iterations=⏱2 keys");
    await wire(page, noise, thermal, "out", "in");
    await fitView(page, height);
    await wire(page, thermal, height, "out"); // replaces the starter noise → height edge
    const evaluated = await logCount(page, "graph.evaluate: ok");
    await page.getByTestId("graph-evaluate").click();
    await expect.poll(() => logCount(page, "graph.evaluate: ok")).toBeGreaterThan(evaluated);
    const stats = page.getByTestId("layer-stats");
    const hashAt0 = (await stats.textContent())?.match(/hash([0-9a-f]+)/)?.[1];
    expect(hashAt0).toBeTruthy();

    // Scrub to t = 1: one journaled set_time (the drag coalesces), the live preview re-evaluates
    // with 300 steps and the published height changes.
    await page.getByTestId("graph-time").evaluate((el) => {
      const input = el as HTMLInputElement;
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      setter?.call(input, "1");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await expect(page.getByTestId("graph-time-value")).toHaveText("1.00");
    await expect(page.getByTestId("log")).toContainText("graph.set_time: ok");
    await expect.poll(async () => (await stats.textContent())?.match(/hash([0-9a-f]+)/)?.[1], { timeout: 30_000 }).not.toBe(hashAt0);
    await page.screenshot({ path: testInfo.outputPath("timeline.png") }); // tutorial image source
    // Undo takes the time back to 0 (journaled like any edit).
    await page.getByTestId("graph-undo").click();
    await expect(page.getByTestId("graph-time-value")).toHaveText("0.00");
    // Ecosystem growth: scatter follows the timeline — nobody is born at t = 0, the population is in by t = 1.
    await page.getByTestId("scatter-mask").selectOption("height");
    await page.getByTestId("scatter-spacing").fill("40");
    await page.getByTestId("scatter-grow").check();
    await page.getByTestId("scatter-place").click();
    await expect(page.getByTestId("log")).toContainText("terrain.scatter: ok");
    await expect(page.getByTestId("scatter-count")).toHaveText("0 instances");
    await page.getByTestId("graph-time").evaluate((el) => {
      const input = el as HTMLInputElement;
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      setter?.call(input, "1");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await expect(page.getByTestId("graph-time-value")).toHaveText("1.00");
    await expect.poll(async () => Number((await page.getByTestId("scatter-count").textContent())?.replace(/\D/g, "") ?? 0), { timeout: 30_000 }).toBeGreaterThan(100);
    await page.screenshot({ path: testInfo.outputPath("timeline-growth.png") }); // tutorial image source
    await page.getByTestId("graph-undo").click();
    await expect(page.getByTestId("graph-time-value")).toHaveText("0.00");
    // Export frames: four heightmaps from t = 0 to 1 through timeline.export, the project's time untouched.
    const frames = page.getByTestId("frames-count");
    await frames.fill("4");
    await page.getByTestId("frames-export").click();
    await expect(page.getByTestId("log")).toContainText("timeline.export: ok", { timeout: 60_000 });
    const frameDir = path.join(ws, "Time.jdproj", "exports", "frames");
    expect(readdirSync(frameDir).filter((f) => f.endsWith(".png")).sort()).toEqual(["height_0000.png", "height_0001.png", "height_0002.png", "height_0003.png"]);
    await expect(page.getByTestId("graph-time-value")).toHaveText("0.00");
    expect(errors).toEqual([]);
  });
});
