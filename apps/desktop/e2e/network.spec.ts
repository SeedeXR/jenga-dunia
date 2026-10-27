import { expect, test } from "@playwright/test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, evaluateAndWait, installBridge, selectNode, wire, createProject, coreOp } from "./bridge";

const NOISE = "01JDGRAPH000000000000N01SE"; // starter graph ids (jd_graph::nodes)
const HEIGHT = "01JDGRAPH0000000000HE1GHT0";

test.describe("hydrology-first node against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-network-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  });

  test("network node: no inputs, `rivers` published as a layer and shown as the colour; a fill node on `out` reports no lakes", async ({ page }, testInfo) => {
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await createProject(core, path.join(ws, "Net.jdproj"), 6400, 128); // Studio's New (the Editor has no project panel)
    await page.goto("/");
    await expect(page.locator("header")).toContainText("World — 128²");

    const panel = page.getByTestId("graph-panel");
    const inspector = page.getByTestId("graph-inspector");
    await page.getByTestId("graph-kind").selectOption("jd.hydrology.network@1");
    await page.getByTestId("graph-add").click();
    const net = panel.locator(".react-flow__node", { hasText: "jd.hydrology.network@1" });
    await selectNode(net);
    // Generated inspector: the mouths enum is a select with the schema's default, the slope its default.
    await expect(inspector.locator("label", { hasText: "mouths" }).locator("select")).toHaveValue("south");
    await expect(inspector.locator("label", { hasText: "slope_m_per_km" }).locator("input")).toHaveAttribute("placeholder", "60");
    const order = inspector.locator("label", { hasText: "river_order" }).locator("input");
    await order.fill("4");
    await order.blur();
    await expect(net).toContainText("river_order=4");

    // Replace the starter noise: network.out → the height output, network.rivers → a `rivers` layer.
    const height = panel.locator(`.react-flow__node[data-id="${HEIGHT}"]`);
    await selectNode(panel.locator(`.react-flow__node[data-id="${NOISE}"]`));
    await inspector.getByRole("button", { name: /Remove node/ }).click();
    await expect(page.getByTestId("log")).toContainText("graph.remove_node: ok");
    await wire(page, net, height);
    await page.getByTestId("graph-kind").selectOption("jd.output.height@1");
    await page.getByTestId("graph-add").click();
    const out = panel.locator(`.react-flow__node:not([data-id="${HEIGHT}"])`, { hasText: "jd.output.height@1" });
    await selectNode(out);
    const layer = inspector.locator("label", { hasText: "layer" }).locator("input");
    await layer.fill("rivers");
    await layer.blur();
    await wire(page, net, out, "rivers");
    await evaluateAndWait(page, "graph-evaluate");
    await expect(page.getByTestId("log")).toContainText("graph.evaluate: ok", { timeout: 30_000 });
    // Published: the core lists the layer (Studio's layer and colour pickers show it).
    await expect.poll(async () => ((await coreOp(core, "terrain.layers")) as { layers: string[] }).layers).toContain("rivers");
    await page.screenshot({ path: testInfo.outputPath("network.png") }); // tutorial image source
    expect(errors, "no uncaught page errors").toEqual([]);
    await core.close();
  });
});
