// Brush alphas: the palette sets the pattern a stroke carries; a crater stroke records `alpha` and
// raises a rim round a bowl in the evaluated terrain.
import { expect, test } from "@playwright/test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, evaluateAndWait, installBridge } from "./bridge";

test.describe("brush alphas against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-alphas-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  });

  test("a crater stroke records its alpha", async ({ page }) => {
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto("/");
    await page.getByTestId("project-path").fill(path.join(ws, "Alphas.jdproj"));
    await page.getByLabel("Extent (m)").fill("1024");
    await page.getByLabel("Resolution (px)").fill("128");
    await page.getByTestId("new-project").click();
    await expect(page.locator("header")).toContainText("World — 128²");
    await evaluateAndWait(page, "generate");
    await page.getByTestId("sculpt-toggle").click();
    await expect(page.getByTestId("brush-alphas").locator("button")).toHaveCount(7);
    await page.getByTestId("alpha-crater").click();
    await expect(page.getByTestId("alpha-crater")).toHaveAttribute("aria-checked", "true");
    const vp = (await page.getByTestId("viewport").boundingBox())!;
    await page.mouse.move(vp.x + vp.width / 2, vp.y + vp.height / 2);
    await page.mouse.down();
    await page.mouse.move(vp.x + vp.width / 2 + 10, vp.y + vp.height / 2, { steps: 3 });
    await page.mouse.up();
    const strokes = async () => {
      const g = (await core.callTool({ name: "graph.list", arguments: {} })).structuredContent as { result: { nodes: { kind: string; params?: { strokes?: { alpha?: string }[] } }[] } };
      return g.result.nodes.find((n) => n.kind === "jd.sculpt.strokes@1")?.params?.strokes?.map((s) => s.alpha ?? "plain") ?? [];
    };
    await expect.poll(strokes).toEqual(["crater"]);
    await evaluateAndWait(page, "graph-evaluate");
    await expect(page.getByTestId("layer-stats")).toBeVisible();
    expect(errors, "no uncaught page errors").toEqual([]);
    await core.close();
  });
});
