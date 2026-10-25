// The layer tree and the icon strip: strip buttons add layers (spliced into the height stack or as
// simulation layers), the tree groups side branches by category, and the search box filters rows.
import { expect, test } from "@playwright/test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, evaluateAndWait, installBridge } from "./bridge";

test.describe("layer tree and icon strip against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-tree-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true });
  });

  test("strip adds, tree groups, search filters", async ({ page }, testInfo) => {
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto("/");
    await page.getByTestId("project-path").fill(path.join(ws, "Tree.jdproj"));
    await page.getByLabel("Extent (m)").fill("1024");
    await page.getByLabel("Resolution (px)").fill("128");
    await page.getByTestId("new-project").click();
    await expect(page.locator("header")).toContainText("World — 128²");
    await evaluateAndWait(page, "generate");
    await expect(page.getByTestId("layer-strip").locator("button")).toHaveCount(10);

    const chain = page.getByTestId("layer-chain").locator("li");
    await page.getByTestId("strip-terrace").click();
    await expect(chain.filter({ hasText: "filter.terrace" })).toHaveCount(1);
    await page.getByTestId("strip-cone").click();
    await expect(chain.filter({ hasText: "shape.primitive" })).toHaveCount(1);
    await expect(chain.filter({ hasText: "shape.primitive" })).toContainText("shape"); // a placed shape is a shape layer, not a filter
    await expect(chain.filter({ hasText: "shape.primitive" }).locator("[data-testid^='layer-remove-']")).toBeEnabled(); // and it can be removed, unlike the generator
    // Snow sits beside the output: a side branch, grouped under Simulation layers.
    await page.getByTestId("strip-snow").click();
    await expect(page.getByTestId("layer-group-simulation")).toContainText("1");
    await expect(page.getByTestId("layer-others")).toContainText("climate.snowpack");

    // Search: only matching rows stay.
    await page.getByTestId("layer-search").fill("terr");
    await expect(chain).toHaveCount(1);
    await expect(chain.first()).toContainText("filter.terrace");
    await page.getByTestId("layer-search").fill("");
    await expect(chain).toHaveCount(3);

    // A group collapses.
    await page.getByTestId("layer-group-stack").click();
    await expect(page.getByTestId("layer-chain")).toHaveCount(0);
    await page.getByTestId("layer-group-stack").click();
    await expect(chain).toHaveCount(3);
    await evaluateAndWait(page, "graph-evaluate");
    await page.screenshot({ path: testInfo.outputPath("tree.png") }); // tutorial image source
    expect(errors, "no uncaught page errors").toEqual([]);
    await core.close();
  });
});
