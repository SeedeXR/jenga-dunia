import { expect, test } from "@playwright/test";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, installBridge, wire } from "./bridge";

test.describe("desktop UI against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true });
  });

  test("create project → generate → viewport shows terrain → export PNG16", async ({ page }, testInfo) => {
    test.slow(); // the software-rendered CI runner bakes the sky, horizon and maps several times over; see hydrology.spec.ts
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
    // The landform readout (terrain.measure) sits under the stats and lists the dominant geomorphons.
    await page.getByTestId("measure").click();
    await expect(page.getByTestId("measure-readout")).toContainText("relief", { timeout: 30_000 });
    await expect(page.getByTestId("measure-readout")).toContainText(/slope \d|%/);
    await expect(page.getByTestId("layer-stats")).toContainText("m");
    // The viewport reports its backend and a non-zero triangle count once the preview mesh is built.
    await expect(page.getByTestId("viewport-info")).toContainText(/viewport: (webgpu|webgl2)/);
    await expect(page.getByTestId("viewport-info")).toContainText(/ [1-9][\d,]* tris/);
    await expect(page.getByTestId("viewport-info")).toContainText(/preview lod \d, [\d.]+ MB in \d+ ms/);
    // PBR texturing: the viewport shades the terrain with baked maps (one texel per sample on a
    // 256² world); the toggle falls back to the height-ramp vertex colours.
    await expect(page.getByTestId("viewport-info")).toContainText("pbr 256×256");
    await page.screenshot({ path: testInfo.outputPath("terrain-pbr.png"), fullPage: false }); // tutorial image source
    await page.getByTestId("pbr-toggle").uncheck();
    await expect(page.getByTestId("viewport-info")).not.toContainText("pbr ");
    await page.getByTestId("pbr-toggle").check();
    await expect(page.getByTestId("viewport-info")).toContainText("pbr 256×256");
    // Sun controls: the default direction is reported; dragging the elevation slider re-lights the scene.
    await expect(page.getByTestId("viewport-info")).toContainText("sun 35°/140°");
    await page.getByTestId("sun-elevation").evaluate((el) => {
      const input = el as HTMLInputElement;
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      setter?.call(input, "12");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await expect(page.getByTestId("viewport-info")).toContainText("sun 12°/140°");
    // The dome is the physical sky, re-baked for the new sun; haze is a slider too.
    await expect(page.getByTestId("viewport-info")).toContainText("sky 512×256");
    // Clouds: the slider re-bakes the dome with cover (the log names the cover it baked with).
    await page.getByTestId("sky-clouds").evaluate((el) => {
      const input = el as HTMLInputElement;
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      setter?.call(input, "0.6");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await expect(page.getByTestId("log")).toContainText("clouds 60 %", { timeout: 30_000 });
    // Fog: a viewport look setting; the header reports the amount.
    await page.getByTestId("sky-fog").evaluate((el) => {
      const input = el as HTMLInputElement;
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      setter?.call(input, "0.5");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await expect(page.getByTestId("viewport-info")).toContainText("fog 50 %");
    await expect(page.getByTestId("sun-turbidity")).toHaveValue("3");
    // The horizon skirt: the noise continued 3× beyond the border, coarse, with a hole for the world.
    await page.getByTestId("horizon-toggle").click();
    await expect(page.getByTestId("viewport-info")).toContainText("horizon 256²");
    // Orbit down to the horizon: the dome above the rim is the physical sky (tutorial image).
    const vp = (await page.getByTestId("viewport").boundingBox())!;
    await page.mouse.move(vp.x + vp.width / 2, vp.y + vp.height / 2);
    await page.mouse.down();
    await page.mouse.move(vp.x + vp.width / 2 - 360, vp.y + vp.height / 2 - 400, { steps: 20 });
    await page.mouse.up();
    await page.screenshot({ path: testInfo.outputPath("terrain-horizon.png"), fullPage: false }); // tutorial image source
    await page.mouse.move(vp.x + vp.width / 2, vp.y + vp.height / 2);
    await page.mouse.down();
    await page.mouse.move(vp.x + vp.width / 2 + 360, vp.y + vp.height / 2 + 400, { steps: 20 });
    await page.mouse.up();
    await page.screenshot({ path: testInfo.outputPath("terrain-low-sun.png"), fullPage: false }); // tutorial image source

    await page.getByTestId("export-path").fill("h.png");
    await page.getByTestId("export").click();
    await expect(page.getByTestId("log")).toContainText("W_EXPORT_LOSS");
    const png = readFileSync(path.join(proj, "exports", "h.png"));
    expect(png.subarray(0, 8)).toEqual(Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]));
    expect(png[24]).toBe(16); // IHDR bit depth

    await page.screenshot({ path: testInfo.outputPath("terrain-generated.png"), fullPage: false });

    // Import: an EXR export comes back through assets/ as a jd.input.heightmap node; wired to the
    // height output and evaluated, the layer stats are the generated terrain's again.
    const statsBefore = await page.getByTestId("layer-stats").innerText();
    await page.getByTestId("export-path").fill("h.exr");
    await page.getByTestId("export").click();
    await expect.poll(() => existsSync(path.join(proj, "exports", "h.exr"))).toBe(true);
    await page.getByTestId("import-path").fill(path.join(proj, "exports", "h.exr"));
    await page.getByTestId("import-asset").click();
    await expect(page.getByTestId("log")).toContainText("project.import_asset: ok");
    expect(existsSync(path.join(proj, "assets", "h.exr"))).toBe(true);
    const panel = page.getByTestId("graph-panel");
    const input = panel.locator(".react-flow__node", { hasText: "jd.input.heightmap@1" });
    await expect(input).toBeVisible();
    await expect(input).toContainText("path=h.exr");
    await page.screenshot({ path: testInfo.outputPath("terrain-import.png"), fullPage: false }); // tutorial image source
    const height = panel.locator(".react-flow__node", { hasText: "jd.output.height@1" });
    await wire(page, input, height);
    await expect(page.getByTestId("log")).toContainText("graph.connect: ok");
    await page.getByTestId("graph-evaluate").click();
    await expect(page.getByTestId("log")).toContainText("graph.evaluate: ok");
    // Same heights (EXR is lossless): min/max and mean match; time and hash lines may differ.
    for (const m of [/([\d.]+ \/ [\d.]+ m)/, /mean\s+([\d.]+ m)/]) {
      await expect(page.getByTestId("layer-stats")).toContainText(statsBefore.match(m)?.[1] ?? "no match");
    }
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
