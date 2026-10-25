import { expect, test } from "@playwright/test";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, installBridge } from "./bridge";

const NOISE = "01JDGRAPH000000000000N01SE"; // starter graph ids (jd_graph::nodes)

test.describe("node graph editor against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-graph-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true });
  });

  test("starter graph → edit param → add node → drag → evaluate → delete; every edit is journaled", async ({ page }, testInfo) => {
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto("/?autosave=0"); // this test reads the journal; autosave would truncate it
    const proj = path.join(ws, "Graph.jdproj");
    await page.getByTestId("project-path").fill(proj);
    await page.getByLabel("Extent (m)").fill("1024");
    await page.getByLabel("Resolution (px)").fill("128"); // small: CI software GL re-renders on every live preview
    await page.getByTestId("new-project").click();
    await expect(page.locator("header")).toContainText("World — 128²");

    // The starter graph (noise → height) is drawn from graph.json.
    const panel = page.getByTestId("graph-panel");
    const noise = panel.locator(`.react-flow__node[data-id="${NOISE}"]`);
    await expect(noise).toBeVisible();
    await expect(panel.locator(".react-flow__node")).toHaveCount(2);
    await expect(panel.locator(".react-flow__edge")).toHaveCount(1);

    // Selecting a node shows its schema-driven inspector; editing a field is graph.set_param.
    await noise.click();
    const inspector = page.getByTestId("graph-inspector");
    await expect(inspector).toContainText("jd.noise.fbm@1");
    // Leaving a field unchanged is not an edit: nothing is journaled (the exact op list below proves it).
    const feature = inspector.locator("label", { hasText: "feature_size_m" }).locator("input");
    await feature.focus();
    await feature.blur();
    const seed = inspector.locator("label", { hasText: "seed" }).locator("input");
    await seed.fill("9");
    await seed.blur();
    await expect(page.getByTestId("log")).toContainText("graph.set_param: ok");
    await expect(noise).toContainText("seed=9");
    // The Noise panel mirrors the node, so Generate will not revert this edit.
    await expect(page.getByTestId("seed")).toHaveValue("9");
    // Live preview: the edit re-evaluates the (complete) starter graph without pressing Evaluate.
    await expect(page.getByTestId("viewport-info")).toContainText(/ [1-9][\d,]* tris/);

    // Add a blur node from the kinds list (graph.kinds drives the dropdown).
    await page.getByTestId("graph-kind").selectOption("jd.filter.blur@1");
    await page.getByTestId("graph-add").click();
    await expect(panel.locator(".react-flow__node")).toHaveCount(3);
    const blur = panel.locator(".react-flow__node", { hasText: "jd.filter.blur@1" });
    // An unconnected required input pauses the live preview with an explanation, not an error.
    await expect(page.getByTestId("log")).toContainText("live preview waiting: input `in`");

    // Wire it in by dragging from the noise output handle to the blur input handle (graph.connect).
    // An unconnected required input is a hard error at evaluate, so the UI must make connecting easy.
    const srcHandle = noise.locator(".react-flow__handle.source");
    const dstHandle = blur.locator(".react-flow__handle.target");
    await dstHandle.hover(); // waits until the canvas has re-fitted and the handles stopped moving
    await srcHandle.hover();
    const src = await srcHandle.boundingBox();
    const dst = await dstHandle.boundingBox();
    if (!src || !dst) throw new Error("handles not laid out");
    await page.mouse.move(src.x + src.width / 2, src.y + src.height / 2);
    await page.mouse.down();
    await page.mouse.move(dst.x + dst.width / 2, dst.y + dst.height / 2, { steps: 10 });
    await page.mouse.up();
    await expect(page.getByTestId("log")).toContainText("graph.connect: ok");
    await expect(panel.locator(".react-flow__edge")).toHaveCount(2);

    // Dragging persists the position (graph.move_node).
    const box = await noise.boundingBox();
    if (!box) throw new Error("noise node has no box");
    await page.mouse.move(box.x + box.width / 2, box.y + 8);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width / 2 + 40, box.y + 48, { steps: 8 });
    await page.mouse.up();
    await expect(page.getByTestId("log")).toContainText("graph.move_node: ok");

    // Evaluate publishes the output layer and refreshes stats + viewport.
    await page.getByTestId("graph-evaluate").click();
    await expect(page.getByTestId("layer-stats")).toBeVisible();
    await expect(page.getByTestId("viewport-info")).toContainText(/ [1-9][\d,]* tris/);
    // The live preview may have shown those already: wait for the explicit evaluation itself, since
    // Undo / Redo are disabled while it runs (busy) and a click on a disabled button is lost.
    await expect(page.getByTestId("log")).toContainText("graph.evaluate: ok");

    // Delete key on the selected blur node removes it and its edge through graph.remove_node.
    await blur.click();
    await expect(blur).toHaveClass(/selected/); // the key goes to the selected node; under load the click lands late
    await blur.press("Delete");
    await expect(panel.locator(".react-flow__node")).toHaveCount(2);
    await expect(panel.locator(".react-flow__edge")).toHaveCount(1);
    await expect(page.getByTestId("log")).toContainText("graph.remove_node: ok");

    // Undo brings the node and its edge back (journaled as graph.restore); redo removes it again.
    await page.getByTestId("graph-undo").click();
    await expect(panel.locator(".react-flow__node")).toHaveCount(3);
    await expect(panel.locator(".react-flow__edge")).toHaveCount(2);
    await page.getByTestId("graph-redo").click();
    await expect(panel.locator(".react-flow__node")).toHaveCount(2);
    await expect(page.getByTestId("graph-redo")).toBeDisabled();

    // Every edit went through the core's WAL before touching the document.
    const journal = readFileSync(path.join(proj, "journal", "ops.ndjson"), "utf8");
    const ops = journal
      .trim()
      .split("\n")
      .map((l) => (JSON.parse(l) as { op: string }).op);
    expect(ops).toEqual(["graph.set_param", "graph.add_node", "graph.connect", "graph.move_node", "graph.remove_node", "graph.restore", "graph.restore"]);

    await page.screenshot({ path: testInfo.outputPath("graph-editor.png") });
    expect(errors, "no uncaught page errors").toEqual([]);
    await core.close();
  });
});
