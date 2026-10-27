// The layer tree and the icon strip: strip buttons add layers (spliced into the height stack or as
// simulation layers), the tree groups side branches by category, and the search box filters rows.
import { expect, test } from "@playwright/test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, evaluateAndWait, installBridge, createProject, coreOp } from "./bridge";

test.describe("layer tree and icon strip against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-tree-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  });

  test("strip adds, tree groups, search filters", async ({ page }, testInfo) => {
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await createProject(core, path.join(ws, "Tree.jdproj"), 1024, 128); // Studio's New (the Editor has no project panel)
    await page.goto("/");
    await expect(page.locator("header")).toContainText("World — 128²");
    await evaluateAndWait(page, "generate");
    const chain = page.getByTestId("layer-chain").locator("li");
    // The stack's + splices a kind in front of the output (Studio's layer strip does the same).
    const add = async (kind: string) => {
      await page.getByTestId("layer-add-kind").selectOption(kind);
      await page.getByTestId("layer-add").click();
    };
    await add("jd.filter.terrace@1");
    await expect(chain.filter({ hasText: "filter.terrace" })).toHaveCount(1);
    // Studio's strip cone: a shape primitive with its cone parameters, spliced by the core.
    await coreOp(core, "graph.splice", { layer: "height", kind: "jd.shape.primitive@1", params: { shape: "cone", center_m: [512, 512], size_m: [340, 340], height_range_m: [0, 160], op: "max" } });
    await page.reload();
    await expect(chain.filter({ hasText: "shape.primitive" })).toHaveCount(1);
    await expect(chain.filter({ hasText: "shape.primitive" })).toContainText("shape"); // a placed shape is a shape layer, not a filter
    await expect(chain.filter({ hasText: "shape.primitive" }).locator("[data-testid^='layer-remove-']")).toBeEnabled(); // and it can be removed, unlike the generator
    // Snow sits beside the output: a side branch, grouped under Simulation layers (added by
    // Studio's Snow button; the Editor re-reads the graph).
    await coreOp(core, "graph.add_simulation", { which: "snow" });
    await page.reload();
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
