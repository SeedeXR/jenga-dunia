import { expect, test } from "@playwright/test";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, installBridge } from "./bridge";

test.describe("sculpting against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-sculpt-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true });
  });

  test("Sculpt inserts a stroke node, a viewport drag records a stroke, the terrain rises, undo removes it", async ({ page }, testInfo) => {
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto("/?autosave=0"); // this test reads the journal; autosave would truncate it
    const proj = path.join(ws, "Sculpt.jdproj");
    await page.getByTestId("project-path").fill(proj);
    await page.getByLabel("Extent (m)").fill("1024");
    await page.getByLabel("Resolution (px)").fill("128");
    await page.getByTestId("new-project").click();
    // Live preview of the starter graph puts terrain in the viewport.
    await expect(page.getByTestId("viewport-info")).toContainText(/ [1-9][\d,]* tris/);
    const before = await page.getByTestId("layer-stats").textContent();

    // One click: a jd.sculpt.strokes node is wired in front of the output and sculpt mode starts.
    await page.getByTestId("sculpt-toggle").click();
    await expect(page.getByTestId("sculpt-toggle")).toContainText("Sculpting");
    await expect(page.getByTestId("graph-panel").locator(".react-flow__node")).toHaveCount(3);
    await expect(page.getByTestId("graph-panel").locator(".react-flow__edge")).toHaveCount(2);
    await page.getByTestId("brush-strength").fill("120");

    // Drag across the middle of the viewport: the pointer is raycast onto the terrain.
    const vp = await page.getByTestId("viewport").boundingBox();
    if (!vp) throw new Error("no viewport");
    const cx = vp.x + vp.width / 2;
    const cy = vp.y + vp.height / 2;
    // Hovering shows the brush ring; the host reports where it sits (layer metres).
    await page.mouse.move(cx, cy);
    await page.mouse.move(cx + 2, cy + 2);
    await expect(page.getByTestId("viewport")).toHaveAttribute("data-brush", /^\d+(\.\d+)?,\d+(\.\d+)?$/);
    const brushAt = (await page.getByTestId("viewport").getAttribute("data-brush"))!.split(",").map(Number);
    expect(brushAt[0]).toBeGreaterThan(0);
    expect(brushAt[0]).toBeLessThan(1024);
    await page.screenshot({ path: testInfo.outputPath("sculpt-brush.png") }); // tutorial image source
    await page.mouse.move(cx - 60, cy);
    await page.mouse.down();
    await page.mouse.move(cx + 60, cy, { steps: 12 });
    await page.mouse.up();
    await expect(page.getByTestId("log")).toContainText("graph.append_param: ok");
    await expect(page.getByTestId("graph-panel").locator(".react-flow__node", { hasText: "jd.sculpt.strokes@1" })).toContainText("strokes=");
    // Live preview re-evaluates: the raised ridge lifts the maximum height.
    await expect(page.getByTestId("layer-stats")).not.toHaveText(before ?? "");

    // Each stroke is one undo step.
    await page.getByTestId("graph-undo").click();
    await expect(page.getByTestId("graph-panel").locator(".react-flow__node", { hasText: "jd.sculpt.strokes@1" })).not.toContainText("strokes=");
    const journal = readFileSync(path.join(proj, "journal", "ops.ndjson"), "utf8")
      .trim()
      .split("\n")
      .map((l) => JSON.parse(l) as { op: string; params: { key?: string; value?: unknown } });
    const stroke = journal.find((e) => e.op === "graph.append_param" && e.params.key === "strokes");
    expect(stroke, `the stroke is journaled with its points; journal ops: ${journal.map((e) => e.op + (e.params.key ? `(${e.params.key})` : "")).join(", ")}`).toBeTruthy();
    const pts = (stroke!.params.value as { points: [number, number][] }).points;
    expect(pts.length).toBeGreaterThan(2);
    for (const [u, v] of pts) {
      expect(u).toBeGreaterThanOrEqual(0);
      expect(u).toBeLessThanOrEqual(1024);
      expect(v).toBeGreaterThanOrEqual(0);
      expect(v).toBeLessThanOrEqual(1024);
    }
    // Orientation: the camera sits south of the terrain (+Z), so a stroke in the lower half of the
    // viewport is nearer the camera = south = small v (layer row 0 is south).
    await page.getByTestId("sculpt-toggle").click(); // back to orbit…
    await page.getByTestId("sculpt-toggle").click(); // …and sculpt again (node already exists)
    // Ctrl + wheel sizes the brush, Shift + wheel sets its strength; the right button orbits
    // without painting (the journal below shows exactly one more stroke).
    await page.mouse.move(cx, cy);
    const radius0 = Number(await page.getByTestId("brush-radius").inputValue());
    await page.keyboard.down("Control");
    await page.mouse.wheel(0, -120);
    await page.keyboard.up("Control");
    await expect(page.getByTestId("brush-radius")).toHaveValue(String(Math.round(radius0 * 1.1)));
    await page.keyboard.down("Shift");
    await page.mouse.wheel(0, 120);
    await page.keyboard.up("Shift");
    expect(Number(await page.getByTestId("brush-strength").inputValue())).toBeLessThan(120);
    await page.mouse.down({ button: "right" });
    await page.mouse.move(cx + 50, cy + 10, { steps: 5 });
    await page.mouse.up({ button: "right" });
    // Lower mode records a negative strength; hardness travels with the stroke.
    await page.getByTestId("brush-mode").selectOption("lower");
    await page.getByTestId("brush-strength").fill("30");
    await page.getByTestId("brush-hardness").fill("0.5");
    const strokesBefore = readFileSync(path.join(proj, "journal", "ops.ndjson"), "utf8").split("\n").filter((l) => l.includes('"strokes"')).length;
    await page.mouse.move(cx - 40, cy + vp.height * 0.28);
    await page.mouse.down();
    await page.mouse.move(cx + 40, cy + vp.height * 0.28, { steps: 8 });
    await page.mouse.up();
    await expect(page.getByTestId("graph-panel").locator(".react-flow__node", { hasText: "jd.sculpt.strokes@1" })).toContainText("strokes=");
    const journal2 = readFileSync(path.join(proj, "journal", "ops.ndjson"), "utf8")
      .trim()
      .split("\n")
      .map((l) => JSON.parse(l) as { op: string; params: { key?: string; value?: unknown } });
    const allStrokes = journal2.filter((e) => e.op === "graph.append_param" && e.params.key === "strokes");
    expect(allStrokes.length, "the right-button orbit painted nothing").toBe(strokesBefore + 1);
    const southStroke = allStrokes.pop()!;
    const south = (southStroke.params.value as { points: [number, number][] }).points;
    for (const [, v] of south) expect(v, "lower half of the screen is south → v < extent/2").toBeLessThan(512);
    expect(southStroke.params.value).toMatchObject({ mode: "raise", strength_m: -30, hardness: 0.5 });
    await page.screenshot({ path: testInfo.outputPath("sculpt.png") });
    expect(errors, "no uncaught page errors").toEqual([]);
    await core.close();
  });
});
