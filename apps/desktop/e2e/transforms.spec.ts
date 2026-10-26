// Region transforms: grid buttons append ops to one jd.sculpt.transforms node and change the terrain;
// a push brush drag appends a grab op.
import { expect, test } from "@playwright/test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, evaluateAndWait, installBridge } from "./bridge";

test.describe("grid and brush transforms against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-transforms-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  });

  test("grid Scale 0 levels the block, Rotate / Duplicate / Align append, a push drag grabs", async ({ page }) => {
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto("/");
    await page.getByTestId("project-path").fill(path.join(ws, "Transforms.jdproj"));
    await page.getByLabel("Extent (m)").fill("1024");
    await page.getByLabel("Resolution (px)").fill("128");
    await page.getByTestId("new-project").click();
    await expect(page.locator("header")).toContainText("World — 128²");
    await evaluateAndWait(page, "generate");
    const view = async () => (await core.callTool({ name: "graph.list", arguments: {} })).structuredContent as { result: { nodes: { kind: string; params?: { ops?: { kind: string }[] } }[] } };
    const ops = async () => (await view()).result.nodes.find((n) => n.kind === "jd.sculpt.transforms@1")?.params?.ops?.map((o) => o.kind) ?? [];

    // Select the middle of the world: the grid's drag over the viewport centre.
    await page.getByTestId("grid-toggle").click();
    const vp = (await page.getByTestId("viewport").boundingBox())!;
    const [cx, cy] = [vp.x + vp.width / 2, vp.y + vp.height / 2];
    await page.mouse.move(cx - 40, cy - 20);
    await page.mouse.down();
    await page.mouse.move(cx + 40, cy + 20, { steps: 8 });
    await page.mouse.up();
    await expect(page.getByTestId("grid-transforms").locator("button").first()).toBeEnabled();

    await page.getByTestId("grid-factor").fill("0");
    await page.getByTestId("grid-scale").click();
    await expect.poll(ops).toEqual(["scale"]);
    await page.getByTestId("grid-rotate").click();
    await page.getByTestId("grid-duplicate").click();
    await page.getByTestId("grid-align").click();
    await expect.poll(ops).toEqual(["scale", "rotate", "duplicate", "align"]);
    await expect(page.getByTestId("layer-chain").locator("li", { hasText: "sculpt.transforms" })).toHaveCount(1);
    await evaluateAndWait(page, "graph-evaluate");
    await expect(page.getByTestId("layer-stats")).toBeVisible();

    // A push brush: one drag appends a grab op with the drag as its vector.
    await page.getByTestId("grid-toggle").click();
    await page.getByTestId("sculpt-toggle").click();
    await page.getByTestId("brush-mode").selectOption("push");
    await expect(page.getByTestId("brush-xform-hint")).toContainText("grab");
    await page.mouse.move(cx, cy);
    await page.mouse.down();
    await page.mouse.move(cx + 60, cy, { steps: 6 });
    await page.mouse.up();
    await expect.poll(ops).toEqual(["scale", "rotate", "duplicate", "align", "push"]);
    expect(errors, "no uncaught page errors").toEqual([]);
    await core.close();
  });
});
