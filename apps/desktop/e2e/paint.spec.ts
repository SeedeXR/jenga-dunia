import { expect, test } from "@playwright/test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, evaluateAndWait, fitView, installBridge, logCount, wire, createProject, coreOp } from "./bridge";

const NOISE = "01JDGRAPH000000000000N01SE"; // starter graph ids (jd_graph::nodes)

/** Painted scatter density: a clamped stroke node on the mask layer, driven by the brush. */
test.describe("painted density against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-paint-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  });

  test("Paint density splices a clamped stroke node in front of the mask output; a stroke changes the instance count", async ({ page }, testInfo) => {
    test.slow();
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await createProject(core, path.join(ws, "Paint.jdproj"), 1280, 128); // Studio's New (the Editor has no project panel)
    await page.goto("/");
    await expect(page.locator("header")).toContainText("World — 128²");

    // A wetness mask published as its own layer.
    const panel = page.getByTestId("graph-panel");
    const noise = panel.locator(`.react-flow__node[data-id="${NOISE}"]`);
    await page.getByTestId("graph-kind").selectOption("jd.hydrology.wetness@1");
    await page.getByTestId("graph-add").click();
    const wet = panel.locator(".react-flow__node", { hasText: "jd.hydrology.wetness@1" });
    await fitView(page, wet);
    await wire(page, noise, wet, "out", "in");
    await page.getByTestId("graph-kind").selectOption("jd.output.height@1");
    await page.getByTestId("graph-add").click();
    const out2 = panel.locator(".react-flow__node", { hasText: "jd.output.height@1" }).last();
    await expect(panel.locator(".react-flow__node", { hasText: "jd.output.height@1" })).toHaveCount(2);
    await fitView(page, out2);
    await out2.click({ force: true });
    const layer = page.getByTestId("graph-inspector").locator("label", { hasText: "layer" }).locator("input");
    await layer.fill("wetness");
    await layer.blur();
    await expect(out2).toContainText("layer=wetness");
    await wire(page, wet, out2, "out");
    const evaluated = await logCount(page, "graph.evaluate: ok");
    await evaluateAndWait(page, "graph-evaluate");
    await expect.poll(() => logCount(page, "graph.evaluate: ok")).toBeGreaterThan(evaluated);

    // Studio's Paint density on the wetness mask: a clamped strokes node in front of its output
    // (graph.sculpt_target), drawn here once the Editor re-reads the graph.
    const scatter = async () => ((await coreOp(core, "terrain.scatter", { mask: "wetness", spacing_m: 20, seed: 1 })) as { count: number }).count;
    const before = await scatter();
    expect(before).toBeGreaterThan(0);
    const { id } = (await coreOp(core, "graph.sculpt_target", { layer: "wetness", kind: "strokes", clamp: [0, 1] })) as { id: string };
    await page.reload();
    const strokes = panel.locator(".react-flow__node", { hasText: "jd.sculpt.strokes@1" });
    await expect(strokes).toHaveCount(1, { timeout: 20_000 });
    await expect(strokes).toContainText("clamp=0…1");
    // One stroke across the middle adds 0.5 of density under the brush: more instances.
    await coreOp(core, "graph.append_param", { id, key: "strokes", value: { points: [[400, 640], [880, 640]], radius_m: 200, strength_m: 0.5, mode: "raise", amount: 0.5, hardness: 0 } });
    await evaluateAndWait(page, "graph-evaluate");
    await expect.poll(scatter, { timeout: 30_000 }).toBeGreaterThan(before);
    await page.screenshot({ path: testInfo.outputPath("paint.png") }); // tutorial image source
    expect(errors).toEqual([]);
  });
});
