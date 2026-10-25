// HDRI import: the sky baked by `sky.bake` is read back by `sky.load_hdri`, drawn on the dome
// (header says `hdri`), and its brightest texel points the sun where the bake put it.
import { expect, test } from "@playwright/test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, evaluateAndWait, installBridge } from "./bridge";

test.describe("HDRI on the dome against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-hdri-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true });
  });

  test("a baked sky loads as an HDRI and 'Use sun' recovers its sun", async ({ page }) => {
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto("/");
    const proj = path.join(ws, "Hdri.jdproj");
    await page.getByTestId("project-path").fill(proj);
    await page.getByLabel("Extent (m)").fill("1024");
    await page.getByLabel("Resolution (px)").fill("128");
    await page.getByTestId("new-project").click();
    await expect(page.locator("header")).toContainText("World — 128²");
    await evaluateAndWait(page, "generate");

    // Bake the physical sky at a known sun, then move the sliders away from it.
    await page.getByTestId("sun-azimuth").fill("200");
    await page.getByTestId("sun-elevation").fill("50");
    await page.getByTestId("export-sky").click();
    await expect(page.getByTestId("log")).toContainText("sky.bake: ok", { timeout: 60_000 });
    await page.getByTestId("sun-azimuth").fill("20");
    await page.getByTestId("sun-elevation").fill("10");

    await page.getByTestId("hdri-path").fill(path.join(proj, "exports", "sky.exr"));
    await page.getByTestId("hdri-load").click();
    await expect(page.getByTestId("log")).toContainText("sky.load_hdri: ok", { timeout: 30_000 });
    await expect(page.getByTestId("viewport-info")).toContainText("hdri");
    await page.getByTestId("hdri-sun").click();
    const az = Number(await page.getByTestId("sun-azimuth").inputValue());
    const el = Number(await page.getByTestId("sun-elevation").inputValue());
    expect(Math.abs(az - 200)).toBeLessThan(12);
    expect(Math.abs(el - 50)).toBeLessThan(12);
    await page.getByTestId("hdri-clear").click();
    await expect(page.getByTestId("viewport-info")).not.toContainText("hdri");
    expect(errors, "no uncaught page errors").toEqual([]);
    await core.close();
  });
});
