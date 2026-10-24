import { expect, test } from "@playwright/test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, installBridge, logCount } from "./bridge";

/** The layer stack against the real core: the chain feeding the height output as rows, "+" splices
 *  a node, the eye bypasses it, the arrows re-order it, the bin removes it and closes the gap. */
test.describe("layer stack against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-stack-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true });
  });

  test("add, bypass, reorder and remove layers from the stack", async ({ page }, testInfo) => {
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto("/");
    await page.getByTestId("project-path").fill(path.join(ws, "Stack.jdproj"));
    await page.getByLabel("Extent (m)").fill("1024");
    await page.getByLabel("Resolution (px)").fill("128");
    await page.getByTestId("new-project").click();
    await expect(page.locator("header")).toContainText("World — 128²");
    await page.getByTestId("generate").click();
    await expect(page.getByTestId("layer-stats")).toBeVisible();
    const chain = page.getByTestId("layer-chain").locator("li");
    await expect(chain).toHaveCount(1); // the noise alone feeds the output
    await expect(chain.first()).toContainText("shape");

    // "+" splices terrace, then thermal, in front of the output: the stack reads top = last applied.
    await page.getByTestId("layer-add-kind").selectOption("jd.filter.terrace@1");
    await page.getByTestId("layer-add").click();
    await expect(chain).toHaveCount(2);
    await page.getByTestId("layer-add-kind").selectOption("jd.erosion.thermal@1");
    await page.getByTestId("layer-add").click();
    await expect(chain).toHaveCount(3);
    await expect(chain.nth(0)).toContainText("erosion.thermal");
    await expect(chain.nth(1)).toContainText("filter.terrace");
    await expect(chain.nth(2)).toContainText("Noise"); // the starter node carries a label
    const view = async () => (await core.callTool({ name: "graph.list", arguments: {} })).structuredContent as { result: { nodes: { id: string; kind: string; bypass: boolean }[]; edges: { from: { node: string }; to: { node: string; port: string } }[] } };
    const thermal = (await view()).result.nodes.find((n) => n.kind === "jd.erosion.thermal@1")!;
    const terrace = (await view()).result.nodes.find((n) => n.kind === "jd.filter.terrace@1")!;

    // The eye bypasses the thermal node (graph.set_bypass): the row dims, the core records the flag.
    await page.getByTestId(`layer-eye-${thermal.id}`).click();
    await expect(page.getByTestId("log")).toContainText("graph.set_bypass: ok");
    expect((await view()).result.nodes.find((n) => n.id === thermal.id)?.bypass).toBe(true);
    await page.getByTestId(`layer-eye-${thermal.id}`).click();
    expect((await view()).result.nodes.find((n) => n.id === thermal.id)?.bypass).toBe(false);

    // Move terrace up: it is applied after thermal now (noise → thermal → terrace → height).
    await page.getByTestId(`layer-up-${terrace.id}`).click();
    await expect(chain.nth(0)).toContainText("filter.terrace");
    await expect(chain.nth(1)).toContainText("erosion.thermal");
    const v = await view();
    const output = v.result.nodes.find((n) => n.kind.startsWith("jd.output.height@"))!;
    expect(v.result.edges.find((e) => e.to.node === output.id && e.to.port === "in")?.from.node).toBe(terrace.id);
    expect(v.result.edges.find((e) => e.to.node === terrace.id && e.to.port === "in")?.from.node).toBe(thermal.id);

    // Remove thermal: the gap closes (noise → terrace → height) and the graph still evaluates.
    const evaluated = await logCount(page, "graph.evaluate: ok");
    await page.getByTestId(`layer-remove-${thermal.id}`).click();
    await expect(page.getByTestId("log")).toContainText("graph.remove_node: ok");
    await expect(chain).toHaveCount(2);
    const v2 = await view();
    expect(v2.result.edges.find((e) => e.to.node === terrace.id && e.to.port === "in")?.from.node).toBe(v2.result.nodes.find((n) => n.kind === "jd.noise.fbm@1")!.id);
    await page.getByTestId("graph-evaluate").click();
    await expect.poll(() => logCount(page, "graph.evaluate: ok"), { timeout: 60_000 }).toBeGreaterThan(evaluated);
    // A side branch (snow) appears under the chain, not in it.
    await page.getByTestId("sim-snow").click();
    await expect(page.getByTestId("layer-others").locator("li")).toHaveCount(1);
    await expect(chain).toHaveCount(2);
    await page.waitForTimeout(300);
    await page.screenshot({ path: testInfo.outputPath("stack.png") }); // tutorial image source
    expect(errors, "no uncaught page errors").toEqual([]);
    await core.close();
  });
});
