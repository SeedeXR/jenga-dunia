import { expect, test } from "@playwright/test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, evaluateAndWait, installBridge, selectNode, wire } from "./bridge";

const NOISE = "01JDGRAPH000000000000N01SE"; // starter graph ids (jd_graph::nodes)
const HEIGHT = "01JDGRAPH0000000000HE1GHT0";

test.describe("climate nodes against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-climate-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true });
  });

  test("orographic rain: node added, wired from the noise, its `rain` output published and shown as the colour layer", async ({ page }, testInfo) => {
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto("/");
    await page.getByTestId("project-path").fill(path.join(ws, "Rain.jdproj"));
    await page.getByLabel("Extent (m)").fill("12800");
    await page.getByLabel("Resolution (px)").fill("128");
    await page.getByTestId("new-project").click();
    await expect(page.locator("header")).toContainText("World — 128²");

    const panel = page.getByTestId("graph-panel");
    const inspector = page.getByTestId("graph-inspector");
    const noise = panel.locator(`.react-flow__node[data-id="${NOISE}"]`);
    await page.getByTestId("graph-kind").selectOption("jd.climate.rain@1");
    await page.getByTestId("graph-add").click();
    const rain = panel.locator(".react-flow__node", { hasText: "jd.climate.rain@1" });
    await selectNode(rain);
    // The inspector is generated from the schema: the wind field shows the node default as its
    // placeholder (unset → the world's shared wind at evaluation), the scale height its default.
    await expect(inspector.locator("label", { hasText: "wind_azimuth_deg" }).locator("input")).toHaveAttribute("placeholder", "270");
    const scale = inspector.locator("label", { hasText: "moisture_scale_m" }).locator("input");
    await expect(scale).toHaveAttribute("placeholder", "2500");
    await scale.fill("1500");
    await scale.blur();
    await expect(rain).toContainText("moisture_scale_m=1500");
    await wire(page, noise, rain);

    // Publish `rain` as its own output layer and colour the terrain by it.
    await page.getByTestId("graph-kind").selectOption("jd.output.height@1");
    await page.getByTestId("graph-add").click();
    const out = panel.locator(`.react-flow__node:not([data-id="${HEIGHT}"])`, { hasText: "jd.output.height@1" });
    await selectNode(out);
    const layer = inspector.locator("label", { hasText: "layer" }).locator("input");
    await layer.fill("rain");
    await layer.blur();
    await wire(page, rain, out, "rain");
    await evaluateAndWait(page, "graph-evaluate");
    await expect(page.getByTestId("log")).toContainText("graph.evaluate: ok", { timeout: 30_000 });
    await expect(page.getByTestId("layer-pick")).toBeVisible();
    await page.getByTestId("color-pick").selectOption("rain");
    await expect(page.getByTestId("viewport-info")).toContainText("colour rain");
    await page.screenshot({ path: testInfo.outputPath("climate-rain.png") }); // tutorial image source
    expect(errors, "no uncaught page errors").toEqual([]);
    await core.close();
  });
});
