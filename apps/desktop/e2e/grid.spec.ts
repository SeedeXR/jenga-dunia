import { expect, test } from "@playwright/test";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, installBridge } from "./bridge";

/** The grid tool against the real core: select cells in the viewport, level them to a height
 *  as one square-stamp stroke, raise them again; the right button orbits meanwhile. */
test.describe("grid tool against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-grid-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true });
  });

  test("cells selected by click and drag are levelled to a height by one undoable stroke", async ({ page }, testInfo) => {
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto("/?autosave=0"); // this test reads the journal; autosave would truncate it
    const proj = path.join(ws, "Grid.jdproj");
    await page.getByTestId("project-path").fill(proj);
    await page.getByLabel("Extent (m)").fill("1024");
    await page.getByLabel("Resolution (px)").fill("128");
    await page.getByTestId("new-project").click();
    await expect(page.getByTestId("viewport-info")).toContainText(/ [1-9][\d,]* tris/);

    // Grid on: the panel appears with no selection; the hovered cell is reported by the host.
    await page.getByTestId("grid-toggle").click();
    await expect(page.getByTestId("grid-count")).toHaveText("0 cells");
    await page.getByTestId("grid-cells").selectOption("16"); // 64 m cells
    const vp = await page.getByTestId("viewport").boundingBox();
    if (!vp) throw new Error("no viewport");
    const cx = vp.x + vp.width / 2;
    const cy = vp.y + vp.height / 2;
    await page.mouse.move(cx, cy);
    await page.mouse.move(cx + 1, cy + 1);
    await expect(page.getByTestId("viewport")).toHaveAttribute("data-cell", /^\d+,\d+$/);
    const [ix, iy] = (await page.getByTestId("viewport").getAttribute("data-cell"))!.split(",").map(Number);
    expect(ix).toBeGreaterThanOrEqual(0);
    expect(ix).toBeLessThan(16);
    expect(iy).toBeGreaterThanOrEqual(0);
    expect(iy).toBeLessThan(16);

    // A click toggles one cell; a drag adds the cells it crosses; a second click toggles it off again.
    await page.mouse.click(cx, cy);
    await expect(page.getByTestId("grid-count")).toHaveText("1 cell");
    await page.mouse.move(cx - 90, cy);
    await page.mouse.down();
    await page.mouse.move(cx + 90, cy, { steps: 12 });
    await page.mouse.up();
    const count = Number((await page.getByTestId("grid-count").textContent())!.split(" ")[0]);
    expect(count).toBeGreaterThan(1);
    await page.mouse.click(cx, cy);
    await expect(page.getByTestId("grid-count")).toHaveText(`${count - 1} cells`);
    await page.mouse.click(cx, cy);
    await expect(page.getByTestId("grid-count")).toHaveText(`${count} cells`);

    // The right button orbits without touching the selection.
    await page.mouse.move(cx, cy - 40);
    await page.mouse.down({ button: "right" });
    await page.mouse.move(cx + 80, cy - 20, { steps: 6 });
    await page.mouse.up({ button: "right" });
    await expect(page.getByTestId("grid-count")).toHaveText(`${count} cells`);

    // Set height: one square-stamp flatten stroke to 600 m (above the 0..500 m noise) over every selected cell: the terrain's maximum is now exactly that.
    await page.getByTestId("grid-height").fill("600");
    await page.getByTestId("grid-set").click();
    await expect(page.getByTestId("log")).toContainText("graph.append_param: ok");
    await expect(page.getByTestId("layer-stats")).toContainText("600.0 m", { timeout: 30_000 });
    await page.waitForTimeout(300);
    await page.screenshot({ path: testInfo.outputPath("grid.png") }); // tutorial image source
    // Raise the same cells by the step: a second stroke, same cells.
    await page.getByTestId("grid-step").fill("25");
    await page.getByTestId("grid-raise").click();
    await expect(page.getByTestId("layer-stats")).toContainText("625.0 m", { timeout: 30_000 });

    const journal = readFileSync(path.join(proj, "journal", "ops.ndjson"), "utf8")
      .trim()
      .split("\n")
      .map((l) => JSON.parse(l) as { op: string; params: { key?: string; value?: Record<string, unknown> } });
    const strokes = journal.filter((e) => e.op === "graph.append_param" && e.params.key === "strokes").map((e) => e.params.value!);
    expect(strokes).toHaveLength(2);
    expect(strokes[0]).toMatchObject({ mode: "flatten", shape: "square", target_m: 600, amount: 1, radius_m: 32 });
    expect((strokes[0].points as number[][]).length).toBe(count);
    expect(strokes[1]).toMatchObject({ mode: "raise", shape: "square", strength_m: 25 });
    // Each grid operation is one undo step.
    await page.getByTestId("graph-undo").click();
    await expect(page.getByTestId("layer-stats")).toContainText("600.0 m", { timeout: 30_000 });
    expect(errors, "no uncaught page errors").toEqual([]);
    await core.close();
  });
});
