import { expect, test } from "@playwright/test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, installBridge, logCount } from "./bridge";

/** One-click simulation layers against the real core: Snow beside the height output, Lava, Debris and
 *  Sand in front of it; masks published as layers, materials added, the viewport coloured by them. */
test.describe("simulation layers against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-sim-layers-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true });
  });

  test("Snow, Lava, Debris and Sand each wire themselves in one click", async ({ page }, testInfo) => {
    test.slow();
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto("/");
    await page.getByTestId("project-path").fill(path.join(ws, "Sim.jdproj"));
    await page.getByLabel("Extent (m)").fill("2048");
    await page.getByLabel("Resolution (px)").fill("128");
    await page.getByTestId("new-project").click();
    await expect(page.locator("header")).toContainText("World — 128²");
    await page.getByTestId("generate").click();
    await expect(page.getByTestId("layer-stats")).toBeVisible();
    const panel = page.getByTestId("graph-panel");

    // Snow: a snowpack node fed by the noise, its cover published as `snow`, a snow material, colour by it.
    await page.getByTestId("sim-snow").click();
    await expect(panel.locator(".react-flow__node", { hasText: "jd.climate.snowpack@1" })).toBeVisible();
    await expect(panel.locator(".react-flow__node")).toHaveCount(4);
    await expect(panel.locator(".react-flow__edge")).toHaveCount(3);
    await expect(page.getByTestId("material-mask-0")).toHaveValue("snow");
    await page.getByTestId("graph-evaluate").click();
    await expect(page.getByTestId("viewport-info")).toContainText("colour snow", { timeout: 60_000 });
    const snow = (await core.callTool({ name: "terrain.stats", arguments: { layer: "snow" } })).structuredContent as { result: { stats: { max: number; mean: number } } };
    expect(snow.result.stats.max).toBeGreaterThan(0.5);
    expect(snow.result.stats.mean).toBeLessThan(0.9); // a snow line inside the world, not a white-out

    // Lava: spliced in front of the height output; heat and lava published; a basalt material.
    await page.getByTestId("sim-lava").click();
    await expect(panel.locator(".react-flow__node", { hasText: "jd.volcanic.lava@1" })).toBeVisible();
    await expect(panel.locator(".react-flow__node")).toHaveCount(7);
    await expect(page.getByTestId("material-mask-1")).toHaveValue("lava");
    const evaluated = await logCount(page, "graph.evaluate: ok");
    await page.getByTestId("graph-evaluate").click();
    await expect.poll(() => logCount(page, "graph.evaluate: ok"), { timeout: 120_000 }).toBeGreaterThan(evaluated);
    await expect(page.getByTestId("viewport-info")).toContainText("colour heat");
    const heat = (await core.callTool({ name: "terrain.stats", arguments: { layer: "heat" } })).structuredContent as { result: { stats: { max: number } } };
    expect(heat.result.stats.max).toBeGreaterThan(0.5);
    await page.waitForTimeout(400);
    await page.screenshot({ path: testInfo.outputPath("simlayers.png") }); // tutorial image source

    // Debris and Sand: each one node in front of the output (debris also publishes its mask).
    await page.getByTestId("sim-debris").click();
    await expect(panel.locator(".react-flow__node", { hasText: "jd.erosion.debris@1" })).toBeVisible();
    await expect(panel.locator(".react-flow__node")).toHaveCount(9);
    await page.getByTestId("sim-sand").click();
    await expect(panel.locator(".react-flow__node", { hasText: "jd.erosion.aeolian@1" })).toBeVisible();
    await expect(panel.locator(".react-flow__node")).toHaveCount(10);
    // The chain still ends at the height output: noise → … → aeolian → height.
    const view = (await core.callTool({ name: "graph.list", arguments: {} })).structuredContent as { result: { nodes: { id: string; kind: string }[]; edges: { from: { node: string }; to: { node: string; port: string } }[] } };
    const height = view.result.nodes.find((n) => n.kind.startsWith("jd.output.height@"))!;
    const feed = view.result.edges.find((e) => e.to.node === height.id && e.to.port === "in")!;
    expect(view.result.nodes.find((n) => n.id === feed.from.node)?.kind).toBe("jd.erosion.aeolian@1");
    expect(errors, "no uncaught page errors").toEqual([]);
    await core.close();
  });
});
