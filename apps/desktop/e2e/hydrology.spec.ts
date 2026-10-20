import { expect, test } from "@playwright/test";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, installBridge } from "./bridge";

const NOISE = "01JDGRAPH000000000000N01SE"; // starter graph ids (jd_graph::nodes)
const HEIGHT = "01JDGRAPH0000000000HE1GHT0";

/** Drag from a node's source handle to another node's target handle (graph.connect). */
async function wire(page: import("@playwright/test").Page, from: import("@playwright/test").Locator, to: import("@playwright/test").Locator, fromHandle = "out") {
  const src = from.locator(`.react-flow__handle.source[data-handleid="${fromHandle}"]`);
  const dst = to.locator(".react-flow__handle.target");
  await dst.hover();
  await src.hover();
  const a = await src.boundingBox();
  const b = await dst.boundingBox();
  if (!a || !b) throw new Error("handles not laid out");
  await page.mouse.move(a.x + a.width / 2, a.y + a.height / 2);
  await page.mouse.down();
  await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2, { steps: 10 });
  await page.mouse.up();
}

test.describe("hydrology nodes against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-hydro-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true });
  });

  test("rivers node: method select (d8 → dinf) in the inspector, wired and evaluated; sea-level node added", async ({ page }, testInfo) => {
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto("/?autosave=2000"); // the project default is 120 s; the test waits for the save
    await page.getByTestId("project-path").fill(path.join(ws, "Hydro.jdproj"));
    await page.getByLabel("Extent (m)").fill("1280");
    await page.getByLabel("Resolution (px)").fill("128");
    await page.getByTestId("new-project").click();
    await expect(page.locator("header")).toContainText("World — 128²");

    const panel = page.getByTestId("graph-panel");
    const noise = panel.locator(`.react-flow__node[data-id="${NOISE}"]`);
    const height = panel.locator(`.react-flow__node[data-id="${HEIGHT}"]`);
    await expect(noise).toBeVisible();

    // Add a rivers node; its inspector exposes `method` as a select built from the schema's oneOf.
    await page.getByTestId("graph-kind").selectOption("jd.hydrology.rivers@1");
    await page.getByTestId("graph-add").click();
    const rivers = panel.locator(".react-flow__node", { hasText: "jd.hydrology.rivers@1" });
    await rivers.click();
    const inspector = page.getByTestId("graph-inspector");
    await expect(inspector).toContainText("jd.hydrology.rivers@1");
    // f32 defaults are shown rounded (0.55, not 0.550000011920929).
    await expect(inspector.locator("label", { hasText: "threshold" }).locator("input")).toHaveAttribute("placeholder", "0.55");
    const method = inspector.locator("label", { hasText: "method" }).locator("select");
    await expect(method).toHaveValue("d8");
    await expect(method.locator("option")).toHaveText(["d8", "dinf", "mfd"]);
    await method.selectOption("dinf");
    await expect(page.getByTestId("log")).toContainText("graph.set_param: ok");
    await expect(rivers).toContainText("method=dinf");
    await page.screenshot({ path: testInfo.outputPath("hydrology-inspector.png") }); // tutorial image source

    // Wire noise → rivers (in) and rivers.out → height output, replacing the starter edge, then evaluate.
    await wire(page, noise, rivers);
    await expect(page.getByTestId("log")).toContainText("graph.connect: ok");
    await wire(page, rivers, height, "out");
    await expect(panel.locator(".react-flow__edge")).toHaveCount(2);
    await page.getByTestId("graph-evaluate").click();
    await expect(page.getByTestId("layer-stats")).toBeVisible();
    await expect(page.getByTestId("viewport-info")).toContainText(/ [1-9][\d,]* tris/);

    // A second output publishes the rivers mask; the header can then colour the terrain by it.
    await page.getByTestId("graph-kind").selectOption("jd.output.height@1");
    await page.getByTestId("graph-add").click();
    const out2 = panel.locator(`.react-flow__node:not([data-id="${HEIGHT}"])`, { hasText: "jd.output.height@1" });
    await out2.click();
    const layerInput = inspector.locator("label", { hasText: "layer" }).locator("input");
    await layerInput.fill("rivers");
    await layerInput.blur();
    await expect(page.getByTestId("log")).toContainText("graph.set_param: ok");
    await wire(page, rivers, out2, "rivers");
    await page.getByTestId("graph-evaluate").click();
    await expect(page.getByTestId("layer-pick")).toBeVisible();
    await page.getByTestId("color-pick").selectOption("rivers");
    await expect(page.getByTestId("viewport-info")).toContainText("colour rivers");
    await page.screenshot({ path: testInfo.outputPath("hydrology-colour.png") }); // tutorial image source
    await page.getByTestId("color-pick").selectOption("");
    await expect(page.getByTestId("viewport-info")).not.toContainText("colour rivers");

    // Sea level is in the kinds list too, with its two outputs.
    await page.getByTestId("graph-kind").selectOption("jd.hydrology.sea_level@1");
    await page.getByTestId("graph-add").click();
    const sea = panel.locator(".react-flow__node", { hasText: "jd.hydrology.sea_level@1" });
    await expect(sea.locator('.react-flow__handle.source[data-handleid="ocean"]')).toBeAttached();
    await expect(sea.locator('.react-flow__handle.source[data-handleid="depth"]')).toBeAttached();
    await sea.click();
    // The viewport draws a sea plane at the node's level; editing level_m moves it.
    await expect(page.getByTestId("viewport-info")).toContainText("sea 0 m");
    const level = inspector.locator("label", { hasText: "level_m" }).locator("input");
    await level.fill("250");
    await level.blur();
    await expect(page.getByTestId("log")).toContainText("graph.set_param: ok");
    await expect(page.getByTestId("viewport-info")).toContainText("sea 250 m");
    await page.screenshot({ path: testInfo.outputPath("hydrology-sea.png") }); // tutorial image source

    // Autosave (interval overridden to 2 s here): after the last edit the project is saved — journal truncated, documents current.
    await expect(page.getByTestId("log")).toContainText("project.save: ok", { timeout: 10_000 });
    await expect(page.getByTestId("project-journal")).toHaveText("0");
    await expect(page.getByTestId("project-dirty")).toHaveText("false");
    const saved = JSON.parse(readFileSync(path.join(ws, "Hydro.jdproj", "graph.json"), "utf8")) as { nodes: Record<string, { kind: string; params?: { level_m?: number } }> };
    expect(Object.values(saved.nodes).some((n) => n.kind.startsWith("jd.hydrology.sea_level@") && n.params?.level_m === 250)).toBe(true);
    expect(readFileSync(path.join(ws, "Hydro.jdproj", "journal", "ops.ndjson"), "utf8")).toBe("");

    expect(errors, "no uncaught page errors").toEqual([]);
    await core.close();
  });
});
