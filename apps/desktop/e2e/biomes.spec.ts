import { expect, test } from "@playwright/test";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, installBridge, logCount, wire } from "./bridge";

const NOISE = "01JDGRAPH000000000000N01SE";
const HEIGHT = "01JDGRAPH0000000000HE1GHT0";

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
    expect(last).toEqual([{ name: "scree", height_m: null, slope: [0.6, 2], wetness: null, mask: null }]);

    // Snow cover: a climate node fed from the noise, published as a second output and used to
    // colour the terrain — a 0..1 mask like any other. The unwired classify node would make the
    // evaluation fail (E_GRAPH_MISSING_INPUT), so it goes first.
    await node.click();
    await inspector.getByRole("button", { name: /Remove node/ }).click();
    await expect(page.getByTestId("log")).toContainText("graph.remove_node: ok");
    const panel = page.getByTestId("graph-panel");
    const noise = panel.locator(`.react-flow__node[data-id="${NOISE}"]`);
    await page.getByTestId("graph-kind").selectOption("jd.climate.snow@1");
    await page.getByTestId("graph-add").click();
    const snow = panel.locator(".react-flow__node", { hasText: "jd.climate.snow@1" });
    await snow.click();
    const line = inspector.locator("label", { hasText: "snow_line_m" }).locator("input");
    await expect(line).toHaveAttribute("placeholder", "800");
    await line.fill("300");
    await line.blur();
    await expect(snow).toContainText("snow_line_m=300");
    await wire(page, noise, snow);
    await page.getByTestId("graph-kind").selectOption("jd.output.height@1");
    await page.getByTestId("graph-add").click();
    const out2 = panel.locator(`.react-flow__node:not([data-id="${HEIGHT}"])`, { hasText: "jd.output.height@1" });
    await out2.click();
    const layerInput = inspector.locator("label", { hasText: "layer" }).locator("input");
    await layerInput.fill("snow");
    await layerInput.blur();
    await wire(page, snow, out2, "snow");
    await page.getByTestId("graph-evaluate").click();
    await expect(page.getByTestId("layer-pick")).toBeVisible();
    await page.getByTestId("color-pick").selectOption("snow");
    await expect(page.getByTestId("viewport-info")).toContainText("colour snow");
    await page.screenshot({ path: testInfo.outputPath("biomes-snow.png") }); // tutorial image source

    // Materials: a white layer weighted by the snow mask re-bakes the viewport's PBR maps.
    const baked = await logCount(page, "terrain.textures: ok");
    await page.getByTestId("material-add").click();
    await page.getByTestId("material-mask-0").selectOption("snow");
    await page.getByTestId("material-color-0").fill("#f4f6fa");
    await expect.poll(() => logCount(page, "terrain.textures: ok"), { timeout: 30_000 }).toBeGreaterThan(baked);
    await page.getByTestId("color-pick").selectOption("");
    await page.screenshot({ path: testInfo.outputPath("materials.png") }); // tutorial image source
    expect(errors, "no uncaught page errors").toEqual([]);
    await core.close();
  });
});
