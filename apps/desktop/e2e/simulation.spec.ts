import { expect, test } from "@playwright/test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, evaluateAndWait, fitView, installBridge, logCount, selectNode, wire, createProject } from "./bridge";

const NOISE = "01JDGRAPH000000000000N01SE"; // starter graph ids (jd_graph::nodes)
const HEIGHT = "01JDGRAPH0000000000HE1GHT0";

/** The simulation nodes built in the desktop: rockfall debris, then a lava flow from a summit vent. */
test.describe("debris and lava simulations against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-sim-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  });

  test("mountains → debris → lava from the summit, coloured by the lava's glow", async ({ page }, testInfo) => {
    test.slow();
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await createProject(core, path.join(ws, "Volcano.jdproj"), 2048, 128); // Studio's New (the Editor has no project panel)
    await page.goto("/");
    await expect(page.locator("header")).toContainText("World — 128²");
    await evaluateAndWait(page, "generate");

    const panel = page.getByTestId("graph-panel");
    const noise = panel.locator(`.react-flow__node[data-id="${NOISE}"]`);
    const height = panel.locator(`.react-flow__node[data-id="${HEIGHT}"]`);
    const inspector = page.getByTestId("graph-inspector");

    // Debris: both new kinds are in the kinds list (graph.kinds drives it), with schema-driven inspectors.
    await page.getByTestId("graph-kind").selectOption("jd.erosion.debris@1");
    await page.getByTestId("graph-add").click();
    const debris = panel.locator(".react-flow__node", { hasText: "jd.erosion.debris@1" });
    await fitView(page, debris);
    await selectNode(debris);
    await expect(inspector).toContainText("jd.erosion.debris@1");
    const reach = inspector.locator("label", { hasText: "reach_deg" }).locator("input");
    await expect(reach).toHaveAttribute("placeholder", "30");
    const source = inspector.locator("label", { hasText: "source_slope_deg" }).locator("input");
    await source.fill("35"); // the 0..500 m starter noise on 16 m cells has few faces above 45°
    await source.blur();
    await expect(debris).toContainText("source_slope_deg=35");
    await wire(page, noise, debris, "out", "in");

    // Lava: no vents → one on the highest cell; half an hour of eruption keeps the debug core quick.
    await page.getByTestId("graph-kind").selectOption("jd.volcanic.lava@1");
    await page.getByTestId("graph-add").click();
    const lava = panel.locator(".react-flow__node", { hasText: "jd.volcanic.lava@1" });
    await fitView(page, lava);
    await selectNode(lava);
    await expect(inspector).toContainText("jd.volcanic.lava@1");
    await expect(inspector.locator("label", { hasText: "cooling" }).locator("input")).toHaveAttribute("placeholder", "0.3");
    for (const [key, value] of [["eruption_s", "1800"], ["duration_s", "1800"]]) {
      const input = inspector.locator("label", { hasText: key }).locator("input");
      await input.fill(value);
      await input.blur();
      await expect(lava).toContainText(`${key}=${value}`);
    }
    await wire(page, debris, lava, "out", "in");
    await fitView(page, height);
    await wire(page, lava, height, "out"); // replaces the starter noise → height edge

    // Publish the glow as its own layer.
    await page.getByTestId("graph-kind").selectOption("jd.output.height@1");
    await page.getByTestId("graph-add").click();
    const out2 = panel.locator(".react-flow__node", { hasText: "jd.output.height@1" }).last();
    await fitView(page, out2);
    await selectNode(out2);
    const layer = inspector.locator("label", { hasText: "layer" }).locator("input");
    await layer.fill("heat");
    await layer.blur();
    await expect(out2).toContainText("layer=heat");
    await wire(page, lava, out2, "heat");

    const evaluated = await logCount(page, "graph.evaluate: ok");
    await evaluateAndWait(page, "graph-evaluate");
    await expect.poll(() => logCount(page, "graph.evaluate: ok"), { timeout: 120_000 }).toBeGreaterThan(evaluated);

    // The core agrees: lava erupted at the summit and is still molten at the end of the eruption.
    const stats = (await core.callTool({ name: "terrain.stats", arguments: { layer: "heat" } })).structuredContent as { result: { stats: { max: number } } };
    expect(stats.result.stats.max).toBeGreaterThan(0.5);
    await page.screenshot({ path: testInfo.outputPath("simulation.png") }); // tutorial image source
    expect(errors, "no uncaught page errors").toEqual([]);
    await core.close();
  });
});
