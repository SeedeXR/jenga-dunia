import { expect, test } from "@playwright/test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, installBridge, logCount } from "./bridge";

// Detail tiles against the real core: a material layer with detail re-bakes the maps, the core
// returns the tile and the packed weights, and the viewport renders them through the TSL
// material without errors.
test.describe("material detail tiles", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-detail-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true });
  });

  test("a detail layer re-bakes the maps with a tile and the viewport stays error-free", async ({ page }, testInfo) => {
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto("/");
    const proj = path.join(ws, "Detail.jdproj");
    await page.getByTestId("project-path").fill(proj);
    await page.getByLabel("Extent (m)").fill("512");
    await page.getByLabel("Resolution (px)").fill("128");
    await page.getByTestId("new-project").click();
    await expect(page.locator("header")).toContainText("World — 128²");
    await page.getByTestId("generate").click();
    await expect(page.getByTestId("viewport-info")).toContainText("pbr", { timeout: 60_000 });

    const baked = await logCount(page, "terrain.textures: ok");
    await page.getByTestId("material-add").click();
    await page.getByTestId("material-color-0").fill("#8a7a62");
    await expect.poll(() => logCount(page, "terrain.textures: ok"), { timeout: 30_000 }).toBeGreaterThan(baked);
    const withDetail = await logCount(page, "terrain.textures: ok");
    await page.getByTestId("material-detail-0").check();
    await expect.poll(() => logCount(page, "terrain.textures: ok"), { timeout: 30_000 }).toBeGreaterThan(withDetail);
    await expect(page.getByTestId("material-tile-0")).toHaveValue("8");
    await page.getByTestId("material-tile-0").fill("4");
    await page.getByTestId("material-tile-0").press("Tab"); // commit on blur
    await expect.poll(() => logCount(page, "terrain.textures: ok"), { timeout: 60_000 }).toBeGreaterThan(withDetail + 1);

    // The core's answer carries the tile and the packed weights for this stack.
    const r = (await core.callTool({ name: "terrain.textures", arguments: { layer: "height", max_px: 256, materials: [{ albedo: [0.54, 0.48, 0.38], roughness: 0.9, detail: { tile_m: 4 } }] } })).structuredContent as {
      result: { details: { layer: number; tile_m: number; px: number; albedo_png_base64: string }[]; detail_weights_png_base64: string };
    };
    expect(r.result.details).toHaveLength(1);
    expect(r.result.details[0]).toMatchObject({ layer: 0, tile_m: 4, px: 512 });
    expect(r.result.detail_weights_png_base64.length).toBeGreaterThan(100);

    await page.waitForTimeout(500); // a frame with the node material
    await page.screenshot({ path: testInfo.outputPath("materials-detail.png") }); // tutorial image source
    expect(errors, "no uncaught page errors").toEqual([]);
    await core.close();
  });
});
