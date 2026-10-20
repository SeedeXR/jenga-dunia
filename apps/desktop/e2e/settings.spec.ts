import { expect, test } from "@playwright/test";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, installBridge } from "./bridge";

const NOISE = "01JDGRAPH000000000000N01SE"; // starter graph id (jd_graph::nodes)

test.describe("project settings against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-settings-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true });
  });

  test("settings panel edits project.json through project.set_settings; a 1 s autosave interval saves after an edit", async ({ page }, testInfo) => {
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto("/"); // no URL override: the autosave interval comes from the project itself
    const proj = path.join(ws, "Settings.jdproj");
    await page.getByTestId("project-path").fill(proj);
    await page.getByLabel("Extent (m)").fill("1024");
    await page.getByLabel("Resolution (px)").fill("128");
    await page.getByTestId("new-project").click();
    await expect(page.locator("header")).toContainText("World — 128²");

    // Defaults from the core are shown.
    await expect(page.getByTestId("setting-autosave")).toHaveValue("120");
    await expect(page.getByTestId("setting-cache")).toHaveValue("2048");
    await expect(page.getByTestId("setting-lod")).toHaveValue("2");
    await expect(page.getByTestId("setting-memory")).toHaveValue("4");
    // Layout: the side panels resize horizontally and the graph panel vertically (native CSS handles).
    const resizeOf = (id: string) => page.getByTestId(id).evaluate((el) => getComputedStyle(el).resize);
    expect(await resizeOf("panel-left")).toBe("horizontal");
    expect(await resizeOf("panel-right")).toBe("horizontal");
    expect(await resizeOf("panel-graph")).toBe("vertical");
    // Sizes persist: a resized panel comes back at the same width after a reload (browser store).
    await page.getByTestId("panel-left").evaluate((el) => {
      (el as HTMLElement).style.width = "300px";
    });
    await expect.poll(() => page.evaluate(() => localStorage.getItem("jd.layout.left"))).toBe("300");
    await page.reload();
    await expect(page.getByTestId("project-path")).toBeVisible();
    expect(await page.getByTestId("panel-left").evaluate((el) => getComputedStyle(el).width)).toBe("300px");
    await page.getByTestId("project-path").fill(proj);
    await page.getByTestId("open-project").click();
    await expect(page.locator("header")).toContainText("World — 128²");
    await expect(page.getByTestId("wind-azimuth")).toHaveValue("270");
    await expect(page.getByTestId("wind-speed")).toHaveValue("8");

    // Autosave → 1 s: journaled mutation, persisted on save, out-of-range values refused by the core.
    const autosave = page.getByTestId("setting-autosave");
    await autosave.fill("1");
    await autosave.blur();
    await expect(page.getByTestId("log")).toContainText("project.set_settings: ok");
    const cache = page.getByTestId("setting-cache");
    await cache.fill("8");
    await cache.blur();
    await expect(page.getByTestId("log")).toContainText("project.set_settings: E_INPUT_INVALID");
    await expect(page.getByTestId("setting-cache")).toHaveValue("2048");
    // The world's shared wind lives in the same op: an easterly for every wind-driven node.
    const wind = page.getByTestId("wind-azimuth");
    await wind.fill("90");
    await wind.blur();
    await expect(page.getByTestId("log")).toContainText("project.set_settings: ok");
    await expect(page.getByTestId("wind-azimuth")).toHaveValue("90");

    // An edit now autosaves after ~1 s: dirty/journal return to false/0 and project.json holds the setting.
    const panel = page.getByTestId("graph-panel");
    await panel.locator(`.react-flow__node[data-id="${NOISE}"]`).click();
    const seed = page.getByTestId("graph-inspector").locator("label", { hasText: "seed" }).locator("input");
    await seed.fill("5");
    await seed.blur();
    await expect(page.getByTestId("log")).toContainText("graph.set_param: ok");
    await expect(page.getByTestId("log")).toContainText("project.save: ok", { timeout: 8_000 });
    await expect(page.getByTestId("project-journal")).toHaveText("0");
    const doc = JSON.parse(readFileSync(path.join(proj, "project.json"), "utf8")) as { settings: { autosave_interval_s: number; cache_limit_bytes: number }; world: { wind: { azimuth_deg: number; speed_mps: number } } };
    expect(doc.settings.autosave_interval_s).toBe(1);
    expect(doc.settings.cache_limit_bytes).toBe(2147483648);
    expect(doc.world.wind).toEqual({ azimuth_deg: 90, speed_mps: 8 });
    await page.screenshot({ path: testInfo.outputPath("settings.png") }); // tutorial image source

    expect(errors, "no uncaught page errors").toEqual([]);
    await core.close();
  });
});
