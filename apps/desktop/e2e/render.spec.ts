// Rendering extras: post effects show in the header, a still lands in exports/stills/, and a
// two-key flythrough writes its frames through project.write_export.
import { expect, test } from "@playwright/test";
import { existsSync, mkdtempSync, readdirSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, evaluateAndWait, installBridge } from "./bridge";

test.describe("post effects, stills and flythrough against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-render-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true });
  });

  test("bloom and vignette read in the header; Still and Record write PNGs under exports/", async ({ page }) => {
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto("/");
    const proj = path.join(ws, "Render.jdproj");
    await page.getByTestId("project-path").fill(proj);
    await page.getByLabel("Extent (m)").fill("1024");
    await page.getByLabel("Resolution (px)").fill("128");
    await page.getByTestId("new-project").click();
    await expect(page.locator("header")).toContainText("World — 128²");
    await evaluateAndWait(page, "generate");

    await page.getByTestId("post-bloom").fill("0.3");
    await page.getByTestId("post-vignette").fill("0.2");
    await expect(page.getByTestId("viewport-info")).toContainText("post bloom 0.3 · vignette 0.2");
    await page.getByTestId("post-bloom").fill("0");
    await page.getByTestId("post-vignette").fill("0");
    await expect(page.getByTestId("viewport-info")).not.toContainText("post ");

    await page.getByTestId("still-save").click();
    await expect(page.getByTestId("log")).toContainText("still: ", { timeout: 30_000 });
    const stills = path.join(proj, "exports", "stills");
    await expect.poll(() => (existsSync(stills) ? readdirSync(stills).length : 0)).toBeGreaterThan(0);
    const still = readdirSync(stills).map((f) => path.join(stills, f))[0]!;
    expect(statSync(still).size).toBeGreaterThan(1024);

    // Two keys: the view as it is, then after a drag that orbits the camera.
    await page.getByTestId("fly-add").click();
    const vp = (await page.getByTestId("viewport").boundingBox())!;
    await page.mouse.move(vp.x + vp.width / 2, vp.y + vp.height / 2);
    await page.mouse.down({ button: "right" });
    await page.mouse.move(vp.x + vp.width / 2 + 120, vp.y + vp.height / 2 + 20, { steps: 6 });
    await page.mouse.up({ button: "right" });
    await page.getByTestId("fly-add").click();
    await expect(page.getByTestId("fly-count")).toContainText("2 keys");
    await page.getByTestId("fly-frames").fill("3");
    await page.getByTestId("fly-record").click();
    await expect(page.getByTestId("log")).toContainText("flythrough: 3/3 frames", { timeout: 60_000 });
    const frames = readdirSync(path.join(proj, "exports", "flythrough")).sort();
    expect(frames).toEqual(["frame_0000.png", "frame_0001.png", "frame_0002.png"]);
    expect(errors, "no uncaught page errors").toEqual([]);
    await core.close();
  });
});
