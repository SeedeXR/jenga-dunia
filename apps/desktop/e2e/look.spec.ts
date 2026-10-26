import { expect, test } from "@playwright/test";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, evaluateAndWait, installBridge, logCount } from "./bridge";

/** The look — materials, scatter settings, sun — is saved with the project (scene.json → look)
 *  and comes back when the project is reopened; the Export section writes engine folders, meshes
 *  and an HDRI with that look. */
test.describe("look and exports against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-look-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  });

  test("materials, scatter and sun survive save and reopen; engine, mesh and HDRI exports carry them", async ({ page }) => {
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto("/");
    const proj = path.join(ws, "Look.jdproj");
    await page.getByTestId("project-path").fill(proj);
    await page.getByLabel("Extent (m)").fill("1024");
    await page.getByLabel("Resolution (px)").fill("128");
    await page.getByTestId("new-project").click();
    await expect(page.locator("header")).toContainText("World — 128²");
    await evaluateAndWait(page, "generate");
    await expect(page.getByTestId("layer-stats")).toBeVisible();

    // A material layer, a sun and a scatter spacing: each setter journals project.set_look (debounced).
    await page.getByTestId("material-add").click();
    await page.getByTestId("material-color-0").fill("#8a7a62");
    await page.getByTestId("sun-azimuth").fill("200");
    await page.getByTestId("scatter-spacing").fill("25");
    await expect.poll(() => logCount(page, "project.set_look: ok"), { timeout: 15_000 }).toBeGreaterThan(0);
    await page.getByTestId("save-project").click();
    await expect(page.getByTestId("log")).toContainText("project.save: ok");
    const scene = JSON.parse(readFileSync(path.join(proj, "scene.json"), "utf8")) as { look: { materials: { albedo: number[] }[]; sun: { azimuth_deg: number }; scatter: { spacing_m: number } } };
    expect(scene.look.materials).toHaveLength(1);
    expect(scene.look.sun.azimuth_deg).toBe(200);
    expect(scene.look.scatter.spacing_m).toBe(25);

    // Exports with the look: an engine folder (weightmaps for the masks, materials.json), a GLB with the
    // materials baked in, and the sun as an HDRI.
    await page.getByTestId("export-engine").selectOption("unreal");
    await page.getByTestId("export-profile").click();
    await expect(page.getByTestId("log")).toContainText("terrain.export_profile: ok", { timeout: 60_000 });
    expect(existsSync(path.join(proj, "exports", "unreal", "heightmap.png"))).toBe(true);
    expect(existsSync(path.join(proj, "exports", "unreal", "materials.json"))).toBe(true);
    // Tiles: 2 per side writes four engine folders that share one height range, and a manifest.
    await page.getByTestId("export-tiles").fill("2");
    await page.getByTestId("export-engine").selectOption("unity");
    await page.getByTestId("export-profile").click();
    await expect.poll(() => existsSync(path.join(proj, "exports", "unity", "tiles.json")), { timeout: 60_000 }).toBe(true);
    expect(existsSync(path.join(proj, "exports", "unity", "tile_1_1", "heightmap.r16"))).toBe(true);
    await page.getByTestId("export-tiles").fill("1");
    await page.getByTestId("export-mesh").click();
    await expect(page.getByTestId("log")).toContainText("terrain.export_mesh: ok", { timeout: 60_000 });
    expect(existsSync(path.join(proj, "exports", "terrain.glb"))).toBe(true);
    await page.getByTestId("export-sky").click();
    await expect(page.getByTestId("log")).toContainText("sky.bake: ok", { timeout: 60_000 });
    expect(existsSync(path.join(proj, "exports", "sky.exr"))).toBe(true);

    // Reopen in the same session: the panel shows the saved look again.
    await page.getByTestId("project-path").fill(path.join(ws, "Other.jdproj"));
    await page.getByTestId("new-project").click();
    await expect(page.getByTestId("material-mask-0")).toHaveCount(0);
    await page.getByTestId("project-path").fill(proj);
    await page.getByTestId("open-project").click();
    await expect(page.locator("header")).toContainText("World — 128²");
    await expect(page.getByTestId("material-color-0")).toHaveValue("#8a7a62");
    await expect(page.getByTestId("sun-azimuth")).toHaveValue("200");
    await expect(page.getByTestId("scatter-spacing")).toHaveValue("25");
    expect(errors, "no uncaught page errors").toEqual([]);
    await core.close();
  });
});
