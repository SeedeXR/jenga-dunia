// Whole-terrain actions: each button splices a jd.filter.action node in front of the height output;
// a flip keeps the height range, Flatten makes min = max.
import { expect, test } from "@playwright/test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, evaluateAndWait, installBridge } from "./bridge";

test.describe("terrain actions against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-actions-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true });
  });

  test("flip keeps the range, flatten levels it, each action is a stack row", async ({ page }) => {
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto("/");
    await page.getByTestId("project-path").fill(path.join(ws, "Actions.jdproj"));
    await page.getByLabel("Extent (m)").fill("1024");
    await page.getByLabel("Resolution (px)").fill("128");
    await page.getByTestId("new-project").click();
    await expect(page.locator("header")).toContainText("World — 128²");
    await evaluateAndWait(page, "generate");
    const stats = async () => (await page.getByTestId("layer-stats").textContent()) ?? "";
    const range = (t: string) => /([-\d.]+) \/ ([-\d.]+) m/.exec(t)!.slice(1, 3).map(Number);
    const before = range(await stats());

    await page.getByTestId("action-flip_x").click();
    await expect(page.getByTestId("layer-chain").locator("li", { hasText: "filter.action" })).toHaveCount(1);
    await evaluateAndWait(page, "graph-evaluate");
    expect(range(await stats())).toEqual(before); // a mirror moves heights, it does not change them

    await page.getByTestId("action-rotate_left").click();
    await expect(page.getByTestId("layer-chain").locator("li", { hasText: "filter.action" })).toHaveCount(2);
    await page.getByTestId("action-flatten").click();
    await evaluateAndWait(page, "graph-evaluate");
    await expect.poll(async () => {
      const [lo, hi] = range(await stats());
      return Math.abs(hi! - lo!);
    }).toBeLessThan(0.01);
    expect(errors, "no uncaught page errors").toEqual([]);
    await core.close();
  });
});
