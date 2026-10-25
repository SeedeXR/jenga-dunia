import { expect, test } from "@playwright/test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, evaluateAndWait, installBridge, logCount, selectNode, wire } from "./bridge";

const HEIGHT = "01JDGRAPH0000000000HE1GHT0";

test.describe("gate pattern against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-gate-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true });
  });

  test("a published layer is read back by jd.input.layer; `only <output>` evaluates just that chain", async ({ page }, testInfo) => {
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto("/");
    await page.getByTestId("project-path").fill(path.join(ws, "Gate.jdproj"));
    await page.getByLabel("Extent (m)").fill("2560");
    await page.getByLabel("Resolution (px)").fill("256");
    await page.getByTestId("new-project").click();
    await expect(page.locator("header")).toContainText("World — 256²");
    const panel = page.getByTestId("graph-panel");
    const inspector = page.getByTestId("graph-inspector");
    // Evaluate the starter graph once: it publishes `height`, the layer the second chain reads
    // back (the CLI quickstart does the same with a coarse `height#lod2`).
    await evaluateAndWait(page, "graph-evaluate");
    await expect(page.getByTestId("log")).toContainText("graph.evaluate: ok", { timeout: 30_000 });

    await page.getByTestId("graph-kind").selectOption("jd.input.layer@1");
    await page.getByTestId("graph-add").click();
    const layer = panel.locator(".react-flow__node", { hasText: "jd.input.layer@1" });
    await selectNode(layer);
    const name = inspector.locator("label", { hasText: "name" }).locator("input");
    await name.fill("height");
    await name.blur();
    await expect(layer).toContainText("name=height");
    await page.getByTestId("graph-kind").selectOption("jd.output.height@1");
    await page.getByTestId("graph-add").click();
    const fine = panel.locator(`.react-flow__node:not([data-id="${HEIGHT}"])`, { hasText: "jd.output.height@1" });
    await selectNode(fine);
    const layerName = inspector.locator("label", { hasText: "layer" }).locator("input");
    await layerName.fill("fine");
    await layerName.blur();
    await wire(page, layer, fine);

    // The selector lists every output layer; choosing one restricts Evaluate to that chain.
    const only = page.getByTestId("graph-only");
    await expect(only.locator("option")).toHaveCount(3); // all, height, fine
    await only.selectOption("fine");
    const before = await logCount(page, "graph.evaluate: ok");
    await evaluateAndWait(page, "graph-evaluate");
    await expect.poll(() => logCount(page, "graph.evaluate: ok"), { timeout: 30_000 }).toBeGreaterThan(before);
    await expect(page.getByTestId("layer-pick")).toBeVisible();
    await expect(page.getByTestId("layer-pick").locator("option", { hasText: "fine" })).toHaveCount(1);
    await page.screenshot({ path: testInfo.outputPath("gate.png") }); // tutorial image source
    expect(errors, "no uncaught page errors").toEqual([]);
    await core.close();
  });
});
