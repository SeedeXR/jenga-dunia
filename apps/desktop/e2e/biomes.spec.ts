import { expect, test } from "@playwright/test";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, installBridge } from "./bridge";

test.describe("biome rules editor against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-biomes-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true });
  });

  test("rules are edited as a table: add a row, type a range, the core journals the array; a bad range is refused", async ({ page }, testInfo) => {
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto("/?autosave=0");
    const proj = path.join(ws, "Biomes.jdproj");
    await page.getByTestId("project-path").fill(proj);
    await page.getByLabel("Extent (m)").fill("1024");
    await page.getByLabel("Resolution (px)").fill("128");
    await page.getByTestId("new-project").click();
    await expect(page.locator("header")).toContainText("World — 128²");

    await page.getByTestId("graph-kind").selectOption("jd.biome.classify@1");
    await page.getByTestId("graph-add").click();
    const node = page.getByTestId("graph-panel").locator(".react-flow__node", { hasText: "jd.biome.classify@1" });
    await node.click();
    const inspector = page.getByTestId("graph-inspector");
    await expect(inspector).toContainText("jd.biome.classify@1");
    // No rules yet: the editor says so and offers a row.
    await expect(inspector.getByTestId("param-rules")).toContainText("none (defaults apply)");
    await inspector.getByTestId("param-rules-add").click();
    await expect(page.getByTestId("log")).toContainText("graph.set_param: ok");
    await expect(inspector.getByTestId("param-rules-row-0")).toBeVisible();
    // Name + slope range → one journaled set_param with the whole array.
    const name = inspector.getByTestId("param-rules-0-name");
    await name.fill("scree");
    await name.blur();
    const slope = inspector.getByTestId("param-rules-0-slope");
    await slope.fill("0.6,2");
    await slope.blur();
    await expect(node).toContainText("rules=");
    await page.screenshot({ path: testInfo.outputPath("biome-rules.png") }); // tutorial image source
    // An inverted range is refused by the core before anything is journaled; the table keeps the last good value.
    const height = inspector.getByTestId("param-rules-0-height_m");
    await height.fill("900,100");
    await height.blur();
    await expect(page.getByTestId("log")).toContainText("graph.set_param: E_INPUT_INVALID");
    const journal = readFileSync(path.join(proj, "journal", "ops.ndjson"), "utf8")
      .trim()
      .split("\n")
      .map((l) => JSON.parse(l) as { op: string; params: { key?: string; value?: unknown } });
    const sets = journal.filter((e) => e.op === "graph.set_param" && e.params.key === "rules");
    const last = sets[sets.length - 1]!.params.value as { name: string; slope: number[] | null; height_m: number[] | null }[];
    expect(last).toEqual([{ name: "scree", height_m: null, slope: [0.6, 2], wetness: null }]);
    expect(errors, "no uncaught page errors").toEqual([]);
    await core.close();
  });
});
