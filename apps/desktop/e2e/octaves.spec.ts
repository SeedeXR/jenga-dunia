// Per-octave strength and general strength: the bars write `octave_weights` on the noise node and
// change the evaluated terrain; strength 0 flattens it to the middle of the height range.
import { expect, test } from "@playwright/test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, evaluateAndWait, installBridge } from "./bridge";

test.describe("octave bars against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-octaves-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true });
  });

  test("a muted octave changes the field; strength 0 levels it", async ({ page }) => {
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto("/");
    await page.getByTestId("project-path").fill(path.join(ws, "Octaves.jdproj"));
    await page.getByLabel("Extent (m)").fill("1024");
    await page.getByLabel("Resolution (px)").fill("128");
    await page.getByTestId("new-project").click();
    await expect(page.locator("header")).toContainText("World — 128²");
    await evaluateAndWait(page, "generate");
    const mean = async () => Number(/mean\s*([-\d.]+)/.exec((await page.getByTestId("layer-stats").textContent()) ?? "")?.[1]);
    const range = async () => {
      const m = /([-\d.]+) \/ ([-\d.]+) m/.exec((await page.getByTestId("layer-stats").textContent()) ?? "")!;
      return Number(m[2]) - Number(m[1]);
    };
    const before = await range();
    await expect(page.getByTestId("octave-bars").locator("input")).toHaveCount(6);

    // Mute the coarsest octave: drag its bar to the bottom and release.
    const bar = page.getByTestId("octave-0");
    await bar.fill("0");
    await bar.dispatchEvent("pointerup");
    await expect(page.getByTestId("log")).toContainText("graph.set_param: ok");
    await evaluateAndWait(page, "graph-evaluate");
    await expect.poll(range).not.toBe(before);

    const strength = page.getByTestId("noise-strength");
    await strength.fill("0");
    await strength.dispatchEvent("pointerup");
    await evaluateAndWait(page, "graph-evaluate");
    await expect.poll(range).toBeLessThan(0.01);
    expect(Math.abs((await mean()) - 250)).toBeLessThan(0.5); // the middle of 0..500 m
    expect(errors, "no uncaught page errors").toEqual([]);
    await core.close();
  });
});
