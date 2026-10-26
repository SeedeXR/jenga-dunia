import { expect, test } from "@playwright/test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, evaluateAndWait, fitView, installBridge, logCount, wire } from "./bridge";

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
    await page.goto("/");
    await page.getByTestId("project-path").fill(path.join(ws, "Paint.jdproj"));
    await page.getByLabel("Extent (m)").fill("1280");
    await page.getByLabel("Resolution (px)").fill("128");
    await page.getByTestId("new-project").click();
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

    // Scatter from it, then paint: the clamped stroke node appears and the brush is live.
    await page.getByTestId("scatter-mask").selectOption("wetness");
    await page.getByTestId("scatter-spacing").fill("20");
    await page.getByTestId("scatter-place").click();
    const info = page.getByTestId("viewport-info");
    await expect(info).toContainText(/\d+ instances/);
    const before = Number((await info.textContent())?.match(/([\d,]+) instances/)?.[1]?.replace(/,/g, ""));
    expect(before).toBeGreaterThan(0);
    await page.getByTestId("scatter-paint").click();
    const strokes = panel.locator(".react-flow__node", { hasText: "jd.sculpt.strokes@1" });
    await expect(strokes).toHaveCount(1, { timeout: 20_000 }); // add_node + two connects + list, under the suite's load
    await expect(strokes).toContainText("clamp=0…1");
    await expect(page.getByTestId("sculpt-toggle")).toContainText("Sculpting");
    // One stroke across the middle of the terrain adds 0.5 of density under the brush.
    const vp = (await page.getByTestId("viewport").boundingBox())!;
    const [cx, cy] = [vp.x + vp.width / 2, vp.y + vp.height / 2];
    await page.mouse.move(cx - 80, cy);
    await page.mouse.down();
    await page.mouse.move(cx + 80, cy, { steps: 12 });
    await page.mouse.up();
    await expect(page.getByTestId("log")).toContainText("graph.append_param: ok");
    await expect.poll(async () => Number((await info.textContent())?.match(/([\d,]+) instances/)?.[1]?.replace(/,/g, "")), { timeout: 30_000 }).toBeGreaterThan(before);
    await page.screenshot({ path: testInfo.outputPath("paint.png") }); // tutorial image source
    expect(errors).toEqual([]);
  });
});
