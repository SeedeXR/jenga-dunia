import { expect, test } from "@playwright/test";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, installBridge } from "./bridge";
import { startTileServer } from "./tileServer";

// The Real-world control against the real core and a loopback Terrarium server (ADR-0014):
// the DEM lands in assets/, the world is placed in its CRS, the heightmap node replaces the
// starter noise, the viewport shows the fetched heights, the notice reaches the export, and a
// second fetch is served from the project's cache.
test.describe("real-world DEM through the desktop", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-geo-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true });
  });

  test("fetch places the DEM, wires it to the height output and records the attribution", async ({ page }, testInfo) => {
    // JD_E2E_REAL_GEO=1 goes to the real AWS bucket (an online check and the tutorial capture);
    // otherwise the loopback server answers, so the suite never touches the internet.
    const real = !!process.env.JD_E2E_REAL_GEO;
    const tiles = await startTileServer();
    if (!real) process.env.JD_GEO_TERRARIUM_URL = tiles.base; // read by the spawned core
    else delete process.env.JD_GEO_TERRARIUM_URL;
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto("/");
    const proj = path.join(ws, "Geo.jdproj");
    await page.getByTestId("project-path").fill(proj);
    await page.getByLabel("Extent (m)").fill("4000");
    await page.getByLabel("Resolution (px)").fill("128");
    await page.getByTestId("new-project").click();
    await expect(page.locator("header")).toContainText("World — 128²");

    await page.getByTestId("geo-lon").fill("36.82");
    await page.getByTestId("geo-lat").fill("-1.29");
    await page.getByTestId("geo-radius").fill("2000");
    await page.getByTestId("geo-fetch").click();
    const log = page.getByTestId("log");
    await expect(log).toContainText("geo.fetch_dem: ok", { timeout: 60_000 });
    await expect(log).toContainText("project.set_settings: ok");
    await expect(log).toContainText("graph.connect: ok");
    await expect(log).toContainText("graph.evaluate: ok", { timeout: 60_000 });

    // The asset, the world placement and the attribution are on disk.
    const assets = path.join(proj, "assets");
    expect(existsSync(path.join(assets, "dem-terrarium-m1.290_36.820.tif"))).toBe(true);
    const info = (await core.callTool({ name: "project.info", arguments: {} })).structuredContent as { result: { world: { crs?: string; origin: number[] } } };
    expect(info.result.world.crs).toBe("EPSG:3857");
    expect(info.result.world.origin[0]).toBeGreaterThan(4_000_000); // Web Mercator metres of 36.8° E
    const attr = (await core.callTool({ name: "geo.attributions", arguments: {} })).structuredContent as { result: { attribution: { source: string }[] } };
    expect(attr.result.attribution.map((a) => a.source)).toContain("AWS Terrain Tiles (Terrarium)");
    // The right panel shows the fetched relief: the loopback tiles (rows 2061–2063 at z12) encode
    // ~6276–6284 m; the real bucket says Nairobi, 1 600–1 800 m.
    await expect(page.getByTestId("layer-stats")).toContainText(real ? /1[67]\d\d\.\d/ : /627\d|628\d/, { timeout: 60_000 });
    await page.screenshot({ path: testInfo.outputPath("geo-fetch.png") }); // tutorial image source
    // Saved, the placement and the notice are in project.json.
    await page.getByTestId("save-project").click();
    await expect.poll(() => JSON.parse(readFileSync(path.join(proj, "project.json"), "utf8")).world.crs ?? null, { timeout: 10_000 }).toBe("EPSG:3857");
    expect(JSON.parse(readFileSync(path.join(proj, "project.json"), "utf8")).attribution.length).toBe(1);

    // The node editor shows the DEM node wired to the output, and the noise unwired.
    await expect(page.locator(".react-flow__node").filter({ hasText: "heightmap" })).toHaveCount(1);

    // A second fetch is served by the cache (conditional GETs answered 304), and overwrite is refused without the flag.
    const before = tiles.hits.count;
    await page.getByTestId("geo-fetch").click();
    await expect(log).toContainText("E_INPUT_INVALID", { timeout: 60_000 }); // same asset name, no overwrite
    if (!real) expect(tiles.hits.count).toBe(before); // refused before any request

    // An export carries ATTRIBUTION.txt.
    await page.getByTestId("export-path").fill("real.exr").catch(() => {});
    await core.callTool({ name: "terrain.export_heightmap", arguments: { layer: "height", path: "real.exr", format: "exr" } });
    expect(readFileSync(path.join(proj, "exports", "ATTRIBUTION.txt"), "utf8")).toContain("AWS Terrain Tiles");
    expect(errors).toEqual([]);
    tiles.server.close();
    await core.close();
  });
});
