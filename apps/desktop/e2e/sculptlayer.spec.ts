// Sculpt-layer settings: a stroke raises the terrain; Height scale 0 hides the layer, a level step
// quantises it, and the settings are params on the strokes node.
import { expect, test } from "@playwright/test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, evaluateAndWait, installBridge } from "./bridge";

test.describe("sculpt-layer settings against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-sculptlayer-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  });

  test("height scale 0 hides the strokes; the node carries the settings", async ({ page }) => {
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto("/");
    await page.getByTestId("project-path").fill(path.join(ws, "Layer.jdproj"));
    await page.getByLabel("Extent (m)").fill("1024");
    await page.getByLabel("Resolution (px)").fill("128");
    await page.getByTestId("new-project").click();
    await expect(page.locator("header")).toContainText("World — 128²");
    await evaluateAndWait(page, "generate");
    const maxOf = async () => Number(/([-\d.]+) \/ ([-\d.]+) m/.exec((await page.getByTestId("layer-stats").textContent()) ?? "")![2]);
    const baseline = await maxOf();

    await page.getByTestId("sculpt-toggle").click();
    await expect(page.getByTestId("layer-settings")).toBeVisible();
    await expect(page.getByTestId("layer-area")).toContainText("whole terrain");
    await page.getByTestId("brush-strength").fill("400");
    const vp = (await page.getByTestId("viewport").boundingBox())!;
    await page.mouse.move(vp.x + vp.width / 2, vp.y + vp.height / 2);
    await page.mouse.down();
    await page.mouse.move(vp.x + vp.width / 2 + 20, vp.y + vp.height / 2, { steps: 3 });
    await page.mouse.up();
    await expect(page.getByTestId("log")).toContainText("graph.append_param: ok");
    await evaluateAndWait(page, "graph-evaluate");
    await expect.poll(maxOf).toBeGreaterThan(baseline + 1);

    const scale = page.getByTestId("layer-height_scale");
    await scale.fill("0");
    await scale.blur();
    await expect(page.getByTestId("log")).toContainText("graph.set_param: ok");
    await evaluateAndWait(page, "graph-evaluate");
    await expect.poll(maxOf).toBeLessThan(baseline + 0.2);
    expect(errors, "no uncaught page errors").toEqual([]);
    await core.close();
  });
});
