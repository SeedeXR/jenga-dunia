import { expect, test } from "@playwright/test";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, installBridge } from "./bridge";

test.describe("desktop UI against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true });
  });

  test("create project → generate → viewport shows terrain → export PNG16", async ({ page }, testInfo) => {
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto("/");
    await expect(page.getByTestId("generate")).toBeDisabled();

    const proj = path.join(ws, "Hills.jdproj");
    await page.getByTestId("project-path").fill(proj);
    // A small world keeps the software-GL render on CI runners fast; the pipeline is identical.
    await page.getByLabel("Extent (m)").fill("1024");
    await page.getByLabel("Resolution (px)").fill("256");
    await page.getByTestId("new-project").click();
    await expect(page.locator("header")).toContainText("World — 256² · 1024 m");
    expect(existsSync(path.join(proj, "project.json"))).toBe(true);

    await page.getByTestId("seed").fill("7");
    await page.getByTestId("generate").click();
    await expect(page.getByTestId("layer-stats")).toBeVisible();
    await expect(page.getByTestId("layer-stats")).toContainText("m");
    // The viewport reports its backend and a non-zero triangle count once the preview mesh is built.
    await expect(page.getByTestId("viewport-info")).toContainText(/viewport: (webgpu|webgl2)/);
    await expect(page.getByTestId("viewport-info")).toContainText(/ [1-9][\d,]* tris/);
    await expect(page.getByTestId("viewport-info")).toContainText(/preview lod \d, [\d.]+ MB in \d+ ms/);
    // Sun controls: the default direction is reported; dragging the elevation slider re-lights the scene.
    await expect(page.getByTestId("viewport-info")).toContainText("sun 35°/140°");
    await page.getByTestId("sun-elevation").evaluate((el) => {
      const input = el as HTMLInputElement;
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      setter?.call(input, "12");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await expect(page.getByTestId("viewport-info")).toContainText("sun 12°/140°");
    await page.screenshot({ path: testInfo.outputPath("terrain-low-sun.png"), fullPage: false }); // tutorial image source

    await page.getByTestId("export-path").fill("h.png");
    await page.getByTestId("export").click();
    await expect(page.getByTestId("log")).toContainText("W_EXPORT_LOSS");
    const png = readFileSync(path.join(proj, "exports", "h.png"));
    expect(png.subarray(0, 8)).toEqual(Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]));
    expect(png[24]).toBe(16); // IHDR bit depth

    await page.screenshot({ path: testInfo.outputPath("terrain-generated.png"), fullPage: false });
    expect(errors, "no uncaught page errors").toEqual([]);
    await core.close();
  });

  test("core errors are shown with code and next step", async ({ page }) => {
    const core = await connectCore(ws);
    await installBridge(page, core);
    await page.goto("/");
    await page.getByTestId("project-path").fill(path.join(ws, "..", "outside.jdproj"));
    await page.getByTestId("new-project").click();
    await expect(page.getByTestId("log")).toContainText("E_PERMISSION");
    await expect(page.getByTestId("log")).toContainText("workspace");
    await core.close();
  });
});
