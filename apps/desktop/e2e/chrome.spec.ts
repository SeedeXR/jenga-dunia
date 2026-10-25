// Viewport chrome: the compass follows the camera's heading, the height ruler spans the layer's
// range, the hints change with the tool, Fit returns to the home view, and the header carries the
// brand mark rather than a placeholder glyph.
import { expect, test } from "@playwright/test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, evaluateAndWait, installBridge } from "./bridge";

test.describe("viewport chrome against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-chrome-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true });
  });

  test("compass, ruler, hints and fit", async ({ page }, testInfo) => {
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto("/");
    // The brand mark (the bird-head line drawing from the brand assets), not a placeholder glyph.
    await expect(page.getByTestId("brand-mark")).toHaveAttribute("src", /jd-mark\.svg$/);
    expect(await page.getByTestId("brand-mark").evaluate((img: HTMLImageElement) => img.naturalWidth)).toBeGreaterThan(0);
    await page.getByTestId("project-path").fill(path.join(ws, "Chrome.jdproj"));
    await page.getByLabel("Extent (m)").fill("1024");
    await page.getByLabel("Resolution (px)").fill("128");
    await page.getByTestId("new-project").click();
    await expect(page.locator("header")).toContainText("World — 128²");
    await evaluateAndWait(page, "generate");

    // Ruler: Max / Min follow the layer stats.
    const stats = await page.getByTestId("layer-stats").textContent();
    const maxM = Number(/([-\d.]+) \/ ([-\d.]+) m/.exec(stats ?? "")?.[2] ?? NaN);
    expect(maxM).toBeGreaterThan(0);
    await expect(page.getByTestId("ruler-max")).toContainText(`Max ${Math.round(maxM)} m`);
    await expect(page.getByTestId("height-ruler")).toBeVisible();

    // Compass: the home heading, then a drag turns it, then Fit brings it back.
    const heading = async () => Number(/(\d+)°/.exec((await page.getByTestId("compass-heading").textContent()) ?? "")?.[1]);
    const home = await heading();
    const vp = (await page.getByTestId("viewport").boundingBox())!;
    await page.mouse.move(vp.x + vp.width / 2, vp.y + vp.height / 2);
    await page.mouse.down({ button: "right" });
    await page.mouse.move(vp.x + vp.width / 2 + 160, vp.y + vp.height / 2, { steps: 8 });
    await page.mouse.up({ button: "right" });
    await expect.poll(heading).not.toBe(home);
    await page.getByTestId("view-fit").click();
    await expect.poll(heading).toBe(home);

    // The header toolbar runs the same actions as the native menu.
    await expect(page.getByTestId("toolbar").locator("button")).toHaveCount(11);
    await page.getByTestId("tb-save").click();
    await expect(page.getByTestId("log")).toContainText("project.save: ok");

    await page.screenshot({ path: testInfo.outputPath("chrome.png") }); // tutorial image source

    // Hints follow the tool.
    await expect(page.getByTestId("viewport-hints")).toHaveAttribute("data-mode", "orbit");
    await page.getByTestId("grid-toggle").click();
    await expect(page.getByTestId("viewport-hints")).toHaveAttribute("data-mode", "grid");
    await expect(page.getByTestId("viewport-hints")).toContainText("remove cells");
    await page.getByTestId("grid-toggle").click();
    await page.getByTestId("sculpt-toggle").click();
    await expect(page.getByTestId("viewport-hints")).toHaveAttribute("data-mode", "sculpt");
    expect(errors, "no uncaught page errors").toEqual([]);
    await core.close();
  });
});
