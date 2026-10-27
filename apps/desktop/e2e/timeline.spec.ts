import { expect, test } from "@playwright/test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, evaluateAndWait, fitView, installBridge, logCount, selectNode, wire, createProject, layerStats, figure } from "./bridge";

const NOISE = "01JDGRAPH000000000000N01SE"; // starter graph ids (jd_graph::nodes)
const HEIGHT = "01JDGRAPH0000000000HE1GHT0";

/** Keyframed parameters and the timeline slider against the real core. */
test.describe("timeline against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-time-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  });

  test("thermal steps keyed 1 → 300: scrubbing the slider re-evaluates the terrain", async ({ page }, testInfo) => {
    test.slow();
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await createProject(core, path.join(ws, "Time.jdproj"), 1280, 128); // Studio's New (the Editor has no project panel)
    await page.goto("/");
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
    await evaluateAndWait(page, "graph-evaluate");
    await expect.poll(() => logCount(page, "graph.evaluate: ok")).toBeGreaterThan(evaluated);
    const hash = async () => (await layerStats(core, "height")).hash;
    const hashAt0 = await hash();
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
    await expect.poll(hash, { timeout: 30_000 }).not.toBe(hashAt0);
    await figure(page, testInfo, "desktop-timeline", [thermal, page.getByTestId("graph-time")]);
    // Undo takes the time back to 0 (journaled like any edit).
    await page.getByTestId("graph-undo").click();
    await expect(page.getByTestId("graph-time-value")).toHaveText("0.00");
    expect(errors).toEqual([]);
  });
});
