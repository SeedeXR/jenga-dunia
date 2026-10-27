import { expect, test } from "@playwright/test";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, evaluateAndWait, fitView, installBridge, logCount, selectNode, wire, createProject, coreOp, layerStats, figure } from "./bridge";

const NOISE = "01JDGRAPH000000000000N01SE"; // starter graph ids (jd_graph::nodes)
const HEIGHT = "01JDGRAPH0000000000HE1GHT0";

/** Drag from a node's source handle to another node's target handle (graph.connect). */
test.describe("hydrology nodes against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-hydro-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  });

  test("rivers node: method select (d8 → dinf) in the inspector, wired and evaluated; sea-level node added", async ({ page }, testInfo) => {
    // ~70 UI actions with live previews between them: 10 s here, close to the 300 s budget on the
    // software-rendered CI runner (three runs timed out mid-spec with every element present).
    test.slow();
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await createProject(core, path.join(ws, "Hydro.jdproj"), 1280, 128); // Studio's New (the Editor has no project panel)
    await page.goto("/?autosave=2000"); // the project default is 120 s; the test waits for the save
    await expect(page.locator("header")).toContainText("World — 128²");

    const panel = page.getByTestId("graph-panel");
    const noise = panel.locator(`.react-flow__node[data-id="${NOISE}"]`);
    const height = panel.locator(`.react-flow__node[data-id="${HEIGHT}"]`);
    await expect(noise).toBeVisible();

    // Add a rivers node; its inspector exposes `method` as a select built from the schema's oneOf.
    await page.getByTestId("graph-kind").selectOption("jd.hydrology.rivers@1");
    await page.getByTestId("graph-add").click();
    const rivers = panel.locator(".react-flow__node", { hasText: "jd.hydrology.rivers@1" });
    await selectNode(rivers);
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
    await figure(page, testInfo, "desktop-rivers-method", [method, rivers]);

    // Wire noise → rivers (in) and rivers.out → height output, replacing the starter edge, then evaluate.
    await wire(page, noise, rivers, "out", "in"); // the rivers node also has a `drains` input
    await expect(page.getByTestId("log")).toContainText("graph.connect: ok");
    await wire(page, rivers, height, "out");
    await expect(panel.locator(".react-flow__edge")).toHaveCount(2);
    // Wait for this evaluation itself (the buttons are disabled while it runs; a click on a disabled button is lost).
    const evaluatedBefore1 = await logCount(page, "graph.evaluate: ok");
    await evaluateAndWait(page, "graph-evaluate");
    await expect.poll(() => logCount(page, "graph.evaluate: ok")).toBeGreaterThan(evaluatedBefore1);

    // A second output publishes the rivers mask; the header can then colour the terrain by it.
    await page.getByTestId("graph-kind").selectOption("jd.output.height@1");
    await page.getByTestId("graph-add").click();
    const out2 = panel.locator(`.react-flow__node:not([data-id="${HEIGHT}"])`, { hasText: "jd.output.height@1" });
    await selectNode(out2);
    const layerInput = inspector.locator("label", { hasText: "layer" }).locator("input");
    await layerInput.fill("rivers");
    await layerInput.blur();
    await expect(page.getByTestId("log")).toContainText("graph.set_param: ok");
    await wire(page, rivers, out2, "rivers");
    // Wait for this evaluation itself (the buttons are disabled while it runs; a click on a disabled button is lost).
    const evaluatedBefore2 = await logCount(page, "graph.evaluate: ok");
    await evaluateAndWait(page, "graph-evaluate");
    await expect.poll(() => logCount(page, "graph.evaluate: ok")).toBeGreaterThan(evaluatedBefore2);
    // Published: Studio can colour the terrain by it and scatter from it.
    await expect.poll(async () => ((await coreOp(core, "terrain.layers")) as { layers: string[] }).layers).toContain("rivers");

    // Sea level is in the kinds list too, with its two outputs.
    await page.getByTestId("graph-kind").selectOption("jd.hydrology.sea_level@1");
    await page.getByTestId("graph-add").click();
    const sea = panel.locator(".react-flow__node", { hasText: "jd.hydrology.sea_level@1" });
    await expect(sea.locator('.react-flow__handle.source[data-handleid="ocean"]')).toBeAttached();
    await expect(sea.locator('.react-flow__handle.source[data-handleid="depth"]')).toBeAttached();
    await selectNode(sea);
    // Editing level_m moves Studio's sea (its window test checks the plane).
    const level = inspector.locator("label", { hasText: "level_m" }).locator("input");
    await level.fill("250");
    await level.blur();
    await expect(sea).toContainText("level_m=250");
    // Water clarity is a node parameter: a murky 4 m visibility is journaled like any other edit.
    const visibility = inspector.locator("label", { hasText: "visibility_m" }).locator("input");
    await expect(visibility).toHaveAttribute("placeholder", "15");
    await visibility.fill("4");
    await visibility.blur();
    await expect(page.getByTestId("log")).toContainText("graph.set_param: ok");
    await expect(sea).toContainText("visibility_m=4");
    // The wind raises the sea (Studio animates and reports the 9.8 m / 380 m sea of a 20 m/s gale).
    const wind = inspector.locator("label", { hasText: "wind_speed_mps" }).locator("input");
    await expect(wind).toHaveAttribute("placeholder", "8");
    await wind.fill("20");
    await wind.blur();
    await expect(sea).toContainText("wind_speed_mps=20");
    // Seabed: classify the floor under the 250 m sea (sand / rock / mud / reef) and colour by it.
    // The sea-level node has only driven the viewport so far: wire it, or the evaluation refuses
    // (E_GRAPH_MISSING_INPUT) and nothing new is published.
    await fitView(page, sea);
    await wire(page, rivers, sea);
    await expect(page.getByTestId("log")).toContainText("graph.connect: ok");
    await page.getByTestId("graph-kind").selectOption("jd.biome.seabed@1");
    await page.getByTestId("graph-add").click();
    const seabed = panel.locator(".react-flow__node", { hasText: "jd.biome.seabed@1" });
    // New nodes land to the right of the graph, off-canvas by now: once the node exists, fit the
    // view so the handles are on screen (fitting before it appears leaves it off-canvas).
    await fitView(page, seabed);
    await selectNode(seabed);
    const seabedLevel = inspector.locator("label", { hasText: "level_m" }).locator("input");
    await seabedLevel.fill("250");
    await seabedLevel.blur();
    await expect(seabed).toContainText("level_m=250");
    await wire(page, rivers, seabed, "out", "in");
    await page.getByTestId("graph-kind").selectOption("jd.output.height@1");
    await page.getByTestId("graph-add").click();
    const out3 = panel.locator(".react-flow__node", { hasText: "jd.output.height@1" }).last();
    await expect(panel.locator(".react-flow__node", { hasText: "jd.output.height@1" })).toHaveCount(3);
    await fitView(page, out3);
    await selectNode(out3);
    const seabedLayer = inspector.locator("label", { hasText: "layer" }).locator("input");
    await seabedLayer.fill("seabed");
    await seabedLayer.blur();
    await expect(out3).toContainText("layer=seabed");
    await wire(page, seabed, out3, "seabed");
    const evaluated = await logCount(page, "graph.evaluate: ok");
    await evaluateAndWait(page, "graph-evaluate");
    await expect.poll(() => logCount(page, "graph.evaluate: ok")).toBeGreaterThan(evaluated);
    await expect.poll(async () => ((await coreOp(core, "terrain.layers")) as { layers: string[] }).layers).toContain("seabed");
    // Drain the sea plane (the seabed node keeps its own 250 m level) so the classified floor is visible.
    await selectNode(sea);
    await level.fill("0");
    await level.blur();
    await expect(sea).toContainText("level_m=0");
    await page.screenshot({ path: testInfo.outputPath("hydrology-seabed.png") }); // tutorial image source

    // Bathymetry: sediment fills the sand and mud hollows of that floor (rock and reef keep their
    // shape); published as `floor`, it is a height layer the viewer can show with the class colours.
    await page.getByTestId("graph-kind").selectOption("jd.ocean.bathymetry@1");
    await page.getByTestId("graph-add").click();
    const bathy = panel.locator(".react-flow__node", { hasText: "jd.ocean.bathymetry@1" });
    await fitView(page, bathy);
    await selectNode(bathy);
    const bathyLevel = inspector.locator("label", { hasText: "level_m" }).locator("input");
    await bathyLevel.fill("250");
    await bathyLevel.blur();
    await expect(bathy).toContainText("level_m=250");
    await wire(page, rivers, bathy, "out", "in");
    await wire(page, seabed, bathy, "seabed", "seabed");
    await page.getByTestId("graph-kind").selectOption("jd.output.height@1");
    await page.getByTestId("graph-add").click();
    const out4 = panel.locator(".react-flow__node", { hasText: "jd.output.height@1" }).last();
    await expect(panel.locator(".react-flow__node", { hasText: "jd.output.height@1" })).toHaveCount(4);
    await fitView(page, out4);
    await selectNode(out4);
    const floorLayer = inspector.locator("label", { hasText: "layer" }).locator("input");
    await floorLayer.fill("floor");
    await floorLayer.blur();
    await expect(out4).toContainText("layer=floor");
    await wire(page, bathy, out4, "out");
    const evaluated2 = await logCount(page, "graph.evaluate: ok");
    await evaluateAndWait(page, "graph-evaluate");
    await expect.poll(() => logCount(page, "graph.evaluate: ok")).toBeGreaterThan(evaluated2);
    // The mean rose from 229.4 m: the hollows filled.
    await expect.poll(async () => Math.round((await layerStats(core, "floor")).mean * 10) / 10, { timeout: 20_000 }).toBe(230.1);
    await page.screenshot({ path: testInfo.outputPath("hydrology-bathymetry.png") }); // tutorial image source

    // Autosave (interval overridden to 2 s here): after the last edit the project is saved — journal truncated, documents current.
    await expect(page.getByTestId("log")).toContainText("project.save: ok", { timeout: 10_000 });
    await expect.poll(async () => { const p = (await coreOp(core, "project.info")) as { dirty: boolean; pending_journal: number }; return [p.dirty, p.pending_journal]; }, { timeout: 10_000 }).toEqual([false, 0]);
    // Poll the files: "project.save: ok" / dirty=false may come from an earlier autosave (the
    // steps above take longer than the 2 s interval), so wait for the save that holds
    // the level_m edit and leaves the journal empty.
    const savedLevel = () => {
      const saved = JSON.parse(readFileSync(path.join(ws, "Hydro.jdproj", "graph.json"), "utf8")) as { nodes: Record<string, { kind: string; params?: { level_m?: number } }> };
      // The last edit drained the sea to 0 m for the seabed capture; that value must be the saved one.
      return Object.values(saved.nodes).some((n) => n.kind.startsWith("jd.hydrology.sea_level@") && n.params?.level_m === 0);
    };
    await expect.poll(savedLevel, { timeout: 10_000 }).toBe(true);
    await expect.poll(() => readFileSync(path.join(ws, "Hydro.jdproj", "journal", "ops.ndjson"), "utf8"), { timeout: 10_000 }).toBe("");

    expect(errors, "no uncaught page errors").toEqual([]);
    await core.close();
  });
});
