import { expect, test } from "@playwright/test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, evaluateAndWait, installBridge } from "./bridge";

// Docking and the viewport detail option against the real core: side panels collapse to a strip,
// the graph docks beside or under the viewport, both survive a reload through the browser store;
// Preview detail 2048² lifts the 1024² transfer cap (a 2048² world then previews at lod 0, 8 MB).
test.describe("docking and preview detail", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-dock-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true });
  });

  test("panels collapse, the graph docks right, the layout persists, and 2048² preview detail shows lod 0", async ({ page }, testInfo) => {
    test.slow(); // the 2048² half rebuilds an 8.4 M-triangle mesh twice: under the parallel suite each preview fetch alone takes ~30 s
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    // Chrome reports "ResizeObserver loop completed with undelivered notifications" when a collapse
    // reflows the observed panels in one frame; it is benign and not an app error.
    page.on("pageerror", (e) => {
      if (!String(e).includes("ResizeObserver loop")) errors.push(String(e));
    });
    await page.goto("/");
    const proj = path.join(ws, "Dock.jdproj");
    await page.getByTestId("project-path").fill(proj);
    await page.getByLabel("Extent (m)").fill("4096");
    await page.getByLabel("Resolution (px)").fill("2048");
    await page.getByTestId("new-project").click();
    await expect(page.locator("header")).toContainText("World — 2048²");

    // Collapse the project panel: a 24 px strip takes its place, and the choice is stored.
    await page.getByTestId("collapse-left").click();
    await expect(page.getByTestId("panel-left-collapsed")).toBeVisible();
    await expect(page.getByTestId("panel-left")).toHaveCount(0);
    expect(JSON.parse((await page.evaluate(() => localStorage.getItem("jd.layout.dock"))) ?? "{}")).toMatchObject({ left: true });

    // Dock the graph beside the viewport: it becomes a horizontally resizable column right of the canvas.
    await page.getByTestId("dock-graph").click();
    const graph = page.getByTestId("panel-graph");
    expect(await graph.evaluate((el) => getComputedStyle(el).resize)).toBe("horizontal");
    const canvas = await page.locator("main canvas").boundingBox();
    const box = await graph.boundingBox();
    expect(box && canvas && box.x >= canvas.x + canvas.width - 1).toBe(true);

    // Reload: still collapsed, still docked right; then back to the default layout.
    await page.reload();
    await expect(page.getByTestId("panel-left-collapsed")).toBeVisible();
    expect(await page.getByTestId("panel-graph").evaluate((el) => getComputedStyle(el).resize)).toBe("horizontal");
    await page.getByTestId("expand-left").click();
    await expect(page.getByTestId("panel-left")).toBeVisible();
    await page.getByTestId("dock-graph").click();
    expect(await page.getByTestId("panel-graph").evaluate((el) => getComputedStyle(el).resize)).toBe("vertical");
    await page.getByTestId("collapse-right").click();
    await expect(page.getByTestId("panel-right-collapsed")).toBeVisible();
    await page.getByTestId("expand-right").click();
    await expect(page.getByTestId("panel-right")).toBeVisible();

    // Preview detail: the 2048² world previews at lod 1 (2 MB) by default and at lod 0 (8 MB) on 2048².
    // An 8.4 M-triangle mesh is a GPU-class load: CI runners render in software and the page blocks for
    // minutes, so this half runs on developer machines only (the capture below comes from one).
    if (process.env.CI) {
      expect(errors, "no uncaught page errors").toEqual([]);
      await core.close();
      return;
    }
    await page.getByTestId("project-path").fill(proj);
    await page.getByTestId("open-project").click();
    await expect(page.locator("header")).toContainText("World — 2048²");
    await evaluateAndWait(page, "generate");
    await expect(page.getByTestId("viewport-info")).toContainText("preview lod 1, 2.00 MB", { timeout: 120_000 });
    await page.getByTestId("setting-preview-px").selectOption("2048");
    await expect(page.getByTestId("viewport-info")).toContainText("preview lod 0, 8.00 MB", { timeout: 120_000 });
    await expect(page.getByTestId("viewport-info")).toContainText(/8,3\d\d,\d\d\d tris/);
    // The tutorial capture: terrain at 2048², project panel collapsed, graph docked beside the viewport, inspector intact.
    await page.getByTestId("collapse-left").click();
    await page.getByTestId("dock-graph").click();
    await expect(page.getByTestId("panel-right")).toBeVisible();
    const right = await page.getByTestId("panel-right").boundingBox();
    expect(right && right.x + right.width <= 1440 + 1, "the inspector stays on screen beside the docked graph").toBe(true);
    await page.waitForTimeout(400);
    await page.screenshot({ path: testInfo.outputPath("docking.png") }); // tutorial image source
    await page.getByTestId("dock-graph").click();
    await page.getByTestId("expand-left").click();
    expect(await page.evaluate(() => localStorage.getItem("jd.previewMaxPx"))).toBe("2048");
    await page.getByTestId("setting-preview-px").selectOption("1024");
    await expect(page.getByTestId("viewport-info")).toContainText("preview lod 1, 2.00 MB", { timeout: 120_000 });
    expect(errors, "no uncaught page errors").toEqual([]);
    await core.close();
  });
});
