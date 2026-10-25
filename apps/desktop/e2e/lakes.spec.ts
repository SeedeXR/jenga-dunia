// Lakes drawn as water: a fill node's `lakes` depth layer, published under that name, is fetched
// beside the height preview and drawn as standing water on the terrain; the header counts its cells.
import { expect, test } from "@playwright/test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, evaluateAndWait, fitView, installBridge, selectNode, wire } from "./bridge";

const NOISE = "01JDGRAPH000000000000N01SE";

test.describe("lakes against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-lakes-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true });
  });

  test("a published `lakes` layer from the fill node is drawn as standing water", async ({ page }) => {
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto("/");
    await page.getByTestId("project-path").fill(path.join(ws, "Lakes.jdproj"));
    await page.getByLabel("Extent (m)").fill("2048");
    await page.getByLabel("Resolution (px)").fill("128");
    await page.getByTestId("new-project").click();
    await expect(page.locator("header")).toContainText("World — 128²");

    const panel = page.getByTestId("graph-panel");
    const noise = panel.locator(`.react-flow__node[data-id="${NOISE}"]`);
    await page.getByTestId("graph-kind").selectOption("jd.hydrology.fill@1");
    await page.getByTestId("graph-add").click();
    const fill = panel.locator(".react-flow__node", { hasText: "jd.hydrology.fill@1" });
    await fitView(page, fill);
    await wire(page, noise, fill, "out", "in");
    await page.getByTestId("graph-kind").selectOption("jd.output.height@1");
    await page.getByTestId("graph-add").click();
    const out2 = panel.locator(".react-flow__node", { hasText: "jd.output.height@1" }).last();
    await expect(panel.locator(".react-flow__node", { hasText: "jd.output.height@1" })).toHaveCount(2);
    await fitView(page, out2);
    await selectNode(out2);
    const layer = page.getByTestId("graph-inspector").locator("label", { hasText: "layer" }).locator("input");
    await layer.fill("lakes");
    await layer.blur();
    await expect(out2).toContainText("layer=lakes");
    await wire(page, fill, out2, "lakes");
    await evaluateAndWait(page, "graph-evaluate");
    // Fractal noise has closed pits: the fill node reports water in them and the viewport draws it.
    await expect(page.getByTestId("viewport-info")).toContainText(/lakes [\d,]+ cells/, { timeout: 30_000 });
    expect(errors, "no uncaught page errors").toEqual([]);
    await core.close();
  });
});
