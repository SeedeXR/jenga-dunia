import { expect, test } from "@playwright/test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, evaluateAndWait, fitView, installBridge, logCount, selectNode, wire, createProject, coreOp, layerStats } from "./bridge";

const NOISE = "01JDGRAPH000000000000N01SE"; // starter graph ids (jd_graph::nodes)
const HEIGHT = "01JDGRAPH0000000000HE1GHT0";

/** A desert world built in the desktop: a low-relief plain, wind-blown dunes, karst sinkholes. */
test.describe("desert landscapes against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-desert-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  });

  test("low plain → aeolian dunes → karst sinkholes, coloured by the sinkhole floors", async ({ page }, testInfo) => {
    test.slow(); // the software-rendered CI runner needs the headroom (see hydrology.spec.ts)
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await createProject(core, path.join(ws, "Desert.jdproj"), 2048, 128); // Studio's New (the Editor has no project panel)
    await page.goto("/");
    await expect(page.locator("header")).toContainText("World — 128²");

    // A desert plain (Studio's Desert plain preset + Generate): broad features, few octaves, 0..40 m.
    for (const [key, value] of Object.entries({ feature_size_m: 1200, octaves: 3, gain: 0.5, height_range_m: [0, 40], seed: 3 })) {
      await coreOp(core, "graph.set_param", { id: NOISE, key, value });
    }
    await page.reload();
    await expect(page.locator("header")).toContainText("World — 128²");
    await evaluateAndWait(page, "graph-evaluate");
    const plain = await layerStats(core, "height");
    expect(plain.max).toBeGreaterThan(30);
    expect(plain.max).toBeLessThanOrEqual(40); // 0..40 m asked; the starter graph (0..500 m) must not overwrite it

    const panel = page.getByTestId("graph-panel");
    const noise = panel.locator(`.react-flow__node[data-id="${NOISE}"]`);
    const height = panel.locator(`.react-flow__node[data-id="${HEIGHT}"]`);
    const inspector = page.getByTestId("graph-inspector");

    // Wind: Werner slabs hop downwind and pile into dunes (60 steps keep the debug core quick).
    await page.getByTestId("graph-kind").selectOption("jd.erosion.aeolian@1");
    await page.getByTestId("graph-add").click();
    const dunes = panel.locator(".react-flow__node", { hasText: "jd.erosion.aeolian@1" });
    await fitView(page, dunes);
    await selectNode(dunes);
    await expect(inspector).toContainText("jd.erosion.aeolian@1");
    const steps = inspector.locator("label", { hasText: "iterations" }).locator("input");
    await steps.fill("60");
    await steps.blur();
    await expect(dunes).toContainText("iterations=60");
    await wire(page, noise, dunes, "out", "in"); // the node also has a `sand` input

    // Sinkholes: dolines on the flat ground between the dunes, published as their own layer.
    await page.getByTestId("graph-kind").selectOption("jd.erosion.karst@1");
    await page.getByTestId("graph-add").click();
    const karst = panel.locator(".react-flow__node", { hasText: "jd.erosion.karst@1" });
    await fitView(page, karst);
    await selectNode(karst);
    const density = inspector.locator("label", { hasText: "density_per_km2" }).locator("input");
    await expect(density).toHaveAttribute("placeholder", "3");
    await density.fill("6");
    await density.blur();
    await expect(karst).toContainText("density_per_km2=6");
    // Dune flanks on 16 m cells run well past the 6° default: let the pits take the interdune flats.
    const slope = inspector.locator("label", { hasText: "max_slope_deg" }).locator("input");
    await slope.fill("15");
    await slope.blur();
    await expect(karst).toContainText("max_slope_deg=15");
    await wire(page, dunes, karst, "out", "in");
    await fitView(page, height);
    await wire(page, karst, height, "out"); // replaces the starter noise → height edge
    await page.getByTestId("graph-kind").selectOption("jd.output.height@1");
    await page.getByTestId("graph-add").click();
    const out2 = panel.locator(".react-flow__node", { hasText: "jd.output.height@1" }).last();
    await expect(panel.locator(".react-flow__node", { hasText: "jd.output.height@1" })).toHaveCount(2);
    await fitView(page, out2);
    await selectNode(out2);
    const layer = inspector.locator("label", { hasText: "layer" }).locator("input");
    await layer.fill("sinkholes");
    await layer.blur();
    await expect(out2).toContainText("layer=sinkholes");
    await wire(page, karst, out2, "sinkholes");

    const evaluated = await logCount(page, "graph.evaluate: ok");
    await evaluateAndWait(page, "graph-evaluate");
    await expect.poll(() => logCount(page, "graph.evaluate: ok"), { timeout: 60_000 }).toBeGreaterThan(evaluated);
    // The sinkhole floors are a mask: pit floors exist (max 1), a small share of the world.
    const pits = await layerStats(core, "sinkholes");
    expect(pits.max).toBe(1);
    expect(pits.mean).toBeLessThan(0.5);
    await page.screenshot({ path: testInfo.outputPath("desert.png") }); // tutorial image source
    expect(errors).toEqual([]);
  });
});
