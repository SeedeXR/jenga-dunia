import { expect, test } from "@playwright/test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, coreOp, createProject, evaluateAndWait, installBridge, layerStats, selectNode } from "./bridge";

/** One-click simulation layers against the real core: Snow beside the height output, Lava, Debris and
 *  Sand in front of it; masks published as layers, materials added, the viewport coloured by them. */
test.describe("simulation layers against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-sim-layers-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  });

  test("Snow, Lava, Debris and Sand each wire themselves in one click", async ({ page }, testInfo) => {
    test.slow();
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await createProject(core, path.join(ws, "Sim.jdproj"), 2048, 128); // Studio's New (the Editor has no project panel)
    await page.goto("/");
    await expect(page.locator("header")).toContainText("World — 128²");
    await evaluateAndWait(page, "generate");
    const panel = page.getByTestId("graph-panel");
    // Studio's simulation buttons add the layers (graph.add_simulation); the Editor re-reads the graph.
    const add = async (which: string) => {
      await coreOp(core, "graph.add_simulation", { which });
      await page.reload();
      await expect(page.locator("header")).toContainText("World — 128²");
    };

    // Snow: a snowpack node fed by the noise, its cover published as `snow`.
    await add("snow");
    const snowpack = panel.locator(".react-flow__node", { hasText: "jd.climate.snowpack@1" });
    await expect(snowpack).toBeVisible();
    await expect(panel.locator(".react-flow__node")).toHaveCount(4);
    await expect(panel.locator(".react-flow__edge")).toHaveCount(3);
    await evaluateAndWait(page, "graph-evaluate");
    const snow = await layerStats(core, "snow");
    expect(snow.max).toBeGreaterThan(0.5);
    expect(snow.mean).toBeLessThan(0.9); // a snow line inside the world, not a white-out

    // A per-node preset applies its values as ordinary edits.
    await selectNode(snowpack);
    await page.getByTestId("node-preset").selectOption("spring thaw");
    await expect(snowpack).toContainText("temperature_c=6", { timeout: 15_000 });

    // Lava: spliced in front of the height output, heat and lava published, every output wired.
    await add("lava");
    await expect(panel.locator(".react-flow__node", { hasText: "jd.volcanic.lava@1" })).toBeVisible();
    await expect(panel.locator(".react-flow__node")).toHaveCount(7);
    await expect(panel.locator(".react-flow__edge")).toHaveCount(6);
    await evaluateAndWait(page, "graph-evaluate");
    expect((await layerStats(core, "heat")).max).toBeGreaterThan(0.5);
    // The molten part glows: a bake with the heat mask carries an emissive map.
    const tx = (await coreOp(core, "terrain.textures", { layer: "height", max_px: 128, materials: [{ mask: "heat", albedo: [1, 0.45, 0.1], roughness: 0.7, emissive: [1, 0.35, 0.05] }] })) as { emissive_png_base64?: string };
    expect(tx.emissive_png_base64?.length ?? 0).toBeGreaterThan(100);
    await page.screenshot({ path: testInfo.outputPath("simlayers.png") }); // tutorial image source

    // Debris and Sand: each one node in front of the output (debris also publishes its mask).
    await add("debris");
    await expect(panel.locator(".react-flow__node", { hasText: "jd.erosion.debris@1" })).toBeVisible();
    await expect(panel.locator(".react-flow__node")).toHaveCount(9);
    await add("sand");
    await expect(panel.locator(".react-flow__node", { hasText: "jd.erosion.aeolian@1" })).toBeVisible();
    await expect(panel.locator(".react-flow__node")).toHaveCount(10);
    // The chain still ends at the height output: noise → … → aeolian → height.
    const view = (await coreOp(core, "graph.list")) as { nodes: { id: string; kind: string }[]; edges: { from: { node: string }; to: { node: string; port: string } }[] };
    const height = view.nodes.find((n) => n.kind.startsWith("jd.output.height@"))!;
    const feed = view.edges.find((e) => e.to.node === height.id && e.to.port === "in")!;
    expect(view.nodes.find((n) => n.id === feed.from.node)?.kind).toBe("jd.erosion.aeolian@1");
    expect(errors, "no uncaught page errors").toEqual([]);
    await core.close();
  });
});
