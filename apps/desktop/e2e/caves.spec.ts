// Caves: add a mine from the Caves panel, place its portal by clicking the terrain, see the cave
// in the viewport through the x-ray, export it — all against the real core.
import { expect, test } from "@playwright/test";
import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, evaluateAndWait, installBridge } from "./bridge";

test.describe("caves against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-caves-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true });
  });

  test("add a mine, click its portal, see it under the x-rayed terrain, export it", async ({ page }, testInfo) => {
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto("/");
    const proj = path.join(ws, "Caves.jdproj");
    await page.getByTestId("project-path").fill(proj);
    await page.getByLabel("Extent (m)").fill("1024");
    await page.getByLabel("Resolution (px)").fill("128");
    await page.getByTestId("new-project").click();
    await expect(page.locator("header")).toContainText("World — 128²");
    await evaluateAndWait(page, "generate");

    await page.getByTestId("cave-kind").selectOption("mine");
    await page.getByTestId("cave-add").click();
    await expect(page.getByTestId("cave-0")).toBeVisible();
    const graph = async () => (await core.callTool({ name: "graph.list", arguments: {} })).structuredContent as { result: { nodes: { id: string; kind: string; params?: { kind?: string; entrances?: [number, number][]; layer?: string } }[] } };
    await expect.poll(async () => (await graph()).result.nodes.filter((n) => n.kind === "jd.cave.system@1").map((n) => n.params?.kind)).toEqual(["mine"]);
    await expect.poll(async () => (await graph()).result.nodes.filter((n) => n.kind === "jd.output.height@1").map((n) => n.params?.layer).sort()).toEqual(["cave_holes", "cave_plan", "height"]);

    // Place the portal: one click on the terrain in placing mode.
    await page.getByTestId("cave-place-0").click();
    await expect(page.getByTestId("viewport-hints")).toHaveAttribute("data-mode", "cave");
    const vp = (await page.getByTestId("viewport").boundingBox())!;
    await page.mouse.move(vp.x + vp.width / 2, vp.y + vp.height / 2);
    await page.mouse.down();
    await page.mouse.move(vp.x + vp.width / 2 + 2, vp.y + vp.height / 2, { steps: 2 });
    await page.mouse.up();
    await expect(page.getByTestId("cave-entrance-0-0")).toBeVisible();
    await page.getByTestId("cave-place-0").click(); // stop placing
    await evaluateAndWait(page, "graph-evaluate");

    // The cave is meshed and drawn; the summary reads its length and entrances.
    await expect(page.getByTestId("cave-summary-0")).toContainText(/\d\.\d\d km · \d entrances?/, { timeout: 30_000 });
    await expect(page.locator("header")).toContainText(/caves [\d,]+ tris/, { timeout: 30_000 });
    await page.getByTestId("caves-xray").fill("0.6");
    await expect(page.locator("header")).toContainText("x-ray 60 %", { timeout: 15_000 });
    await page.screenshot({ path: testInfo.outputPath("caves.png") }); // tutorial image source

    // Remove and re-add an entrance through the list.
    await page.getByTestId("cave-entrance-remove-0-0").click();
    await expect(page.getByTestId("cave-entrance-0-0")).toHaveCount(0);
    await expect.poll(async () => (await graph()).result.nodes.find((n) => n.kind === "jd.cave.system@1")?.params?.entrances ?? []).toEqual([]);

    // Export: GLB under exports/.
    await page.getByTestId("cave-export-0").click();
    await expect.poll(() => existsSync(path.join(proj, "exports", "cave_1.glb")), { timeout: 30_000 }).toBe(true);
    expect(errors, "no uncaught page errors").toEqual([]);
    await core.close();
  });

  test("a karst cave from the layer strip: a branchwork with speleothems under the x-rayed terrain", async ({ page }, testInfo) => {
    test.setTimeout(180_000); // a whole karst system is generated and meshed at every evaluation
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto("/");
    await page.getByTestId("project-path").fill(path.join(ws, "Karst.jdproj"));
    await page.getByLabel("Extent (m)").fill("512");
    await page.getByLabel("Resolution (px)").fill("128");
    await page.getByTestId("new-project").click();
    await expect(page.locator("header")).toContainText("World — 128²");
    await evaluateAndWait(page, "generate");
    await page.getByTestId("strip-cave").click();
    await expect(page.getByTestId("cave-0")).toBeVisible();
    await evaluateAndWait(page, "graph-evaluate");
    await expect(page.getByTestId("cave-summary-0")).toContainText(/Karst \(limestone\) · \d+\.\d\d km · \d+ entrances · \d+ speleothems · water \d+ m/, { timeout: 60_000 });
    await page.getByTestId("caves-xray").fill("0.75");
    await expect(page.locator("header")).toContainText(/caves [\d,]+ tris · x-ray 75 %/, { timeout: 30_000 });
    await page.screenshot({ path: testInfo.outputPath("caves-karst.png") }); // tutorial image source
    // The footprint on the map: the terrain coloured by the caves' plan.
    await page.getByTestId("caves-map").click();
    await expect(page.locator("header")).toContainText("colour cave_plan", { timeout: 30_000 });
    await page.screenshot({ path: testInfo.outputPath("caves-plan.png") }); // tutorial image source
    // Hiding the caves takes them out of the viewport.
    await page.getByTestId("caves-show").uncheck();
    await expect(page.locator("header")).not.toContainText("caves ");
    expect(errors, "no uncaught page errors").toEqual([]);
    await core.close();
  });
});
