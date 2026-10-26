// Studio ↔ desktop parity captures (ADR-0017, owner rule: no visible UI change). Opt-in:
// `JD_PARITY_DIR=<dir> pnpm exec playwright test parity-capture` writes desktop.png for a fixed
// project; `JD_STUDIO_CAPTURE=<dir>/studio.png cargo test -p jd-studio --test window -- --ignored
// capture` renders Studio on the same project (same world, seed and starter graph).
import { expect, test } from "@playwright/test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, evaluateAndWait, installBridge } from "./bridge";

test("parity capture: desktop", async ({ page }) => {
  const out = process.env.JD_PARITY_DIR;
  test.skip(!out, "set JD_PARITY_DIR to write the capture");
  const ws = mkdtempSync(path.join(tmpdir(), "jd-parity-"));
  try {
    const core = await connectCore(ws);
    await installBridge(page, core);
    await page.goto("/");
    await page.getByTestId("project-path").fill(path.join(ws, "Hills.jdproj"));
    await page.getByLabel("Name").first().fill("Hills");
    await page.getByLabel("Extent (m)").fill("4096");
    await page.getByLabel("Resolution (px)").fill("1024");
    await page.getByTestId("new-project").click();
    await evaluateAndWait(page, "generate");
    // The default look: baked PBR maps and the sky preview, both reported in the header.
    await expect(page.getByTestId("viewport-info")).toContainText("pbr", { timeout: 60_000 });
    await expect(page.getByTestId("viewport-info")).toContainText("sky");
    await page.waitForTimeout(1000);
    await page.screenshot({ path: path.join(out!, "desktop.png") });
    // The horizon skirt (JD_PARITY_HORIZON=1).
    if (process.env.JD_PARITY_HORIZON) {
      await page.getByTestId("horizon-toggle").click();
      await expect(page.getByTestId("viewport-info")).toContainText("horizon 256²", { timeout: 30_000 });
      await page.waitForTimeout(1000);
      await page.screenshot({ path: path.join(out!, "desktop-horizon.png") });
    }
    // Scatter (JD_PARITY_SCATTER=1): the height layer as the mask, 40 m spacing, Place.
    if (process.env.JD_PARITY_SCATTER) {
      await page.getByTestId("scatter-mask").selectOption("height");
      await page.getByTestId("scatter-place").click();
      await expect(page.getByTestId("viewport-info")).toContainText(" instances", { timeout: 30_000 });
      await page.waitForTimeout(1000);
      await page.screenshot({ path: path.join(out!, "desktop-scatter.png") });
    }
    // A cave (JD_PARITY_CAVE=1): Add cave (karst), then x-ray 20 %.
    if (process.env.JD_PARITY_CAVE) {
      await page.getByTestId("cave-add").click();
      await expect(page.getByTestId("viewport-info")).toContainText(" · caves ", { timeout: 60_000 });
      await page.getByTestId("caves-xray").fill("0.2");
      await expect(page.getByTestId("viewport-info")).toContainText("x-ray 20 %");
      await page.waitForTimeout(1000);
      await page.screenshot({ path: path.join(out!, "desktop-cave.png") });
    }
    // The Noise section (JD_PARITY_NOISE=1): scrolled to the octave bars.
    if (process.env.JD_PARITY_NOISE) {
      await page.getByTestId("octave-bars").scrollIntoViewIfNeeded();
      await page.waitForTimeout(500);
      await page.screenshot({ path: path.join(out!, "desktop-noise.png") });
    }
    // The post effects (JD_PARITY_POST=1): exposure 1.5, bloom 0.6, vignette 0.5.
    if (process.env.JD_PARITY_POST) {
      await page.getByTestId("post-exposure").fill("1.5");
      await page.getByTestId("post-bloom").fill("0.6");
      await page.getByTestId("post-vignette").fill("0.5");
      await expect(page.getByTestId("viewport-info")).toContainText("post bloom 0.6 · vignette 0.5 · exposure 1.5");
      await page.waitForTimeout(1000);
      await page.screenshot({ path: path.join(out!, "desktop-post.png") });
    }
  } finally {
    rmSync(ws, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  }
});

// The sea (JD_PARITY_SEA=1): the same world with a sea-level node at 180 m on the noise, built in
// the core before the page loads (the desktop adopts the open project on mount).
test("parity capture: desktop sea", async ({ page }) => {
  const out = process.env.JD_PARITY_DIR;
  test.skip(!out || !process.env.JD_PARITY_SEA, "set JD_PARITY_DIR and JD_PARITY_SEA");
  const ws = mkdtempSync(path.join(tmpdir(), "jd-parity-sea-"));
  try {
    const core = await connectCore(ws);
    const call = async (name: string, args: Record<string, unknown>) => {
      const r = await core.callTool({ name, arguments: args });
      const env = r.structuredContent as { status: string; result?: Record<string, unknown> };
      if (env.status !== "ok") throw new Error(`${name}: ${JSON.stringify(env)}`);
      return env.result!;
    };
    const world = { extent_m: [4096, 4096], resolution_px: [1024, 1024], tile_size: 512, halo: 4, origin: [0, 0], seed: 42 };
    await call("project.create", { path: path.join(ws, "Sea.jdproj"), name: "Sea", world });
    const g = (await call("graph.list", {})) as { nodes: { id: string; kind: string }[] };
    const noise = g.nodes.find((n) => n.kind.startsWith("jd.noise."))!;
    const sea = (await call("graph.add_node", { kind: "jd.hydrology.sea_level@1", params: { level_m: 180 } })) as { id: string };
    await call("graph.connect", { from: noise.id, from_port: "out", to: sea.id, to_port: "in" });
    await installBridge(page, core);
    await page.goto("/");
    await expect(page.locator("header")).toContainText("Sea —");
    await evaluateAndWait(page, "graph-evaluate");
    await expect(page.getByTestId("viewport-info")).toContainText("pbr", { timeout: 60_000 });
    await expect(page.getByTestId("viewport-info")).toContainText("sea 180 m");
    await page.waitForTimeout(1000);
    await page.screenshot({ path: path.join(out!, "desktop-sea.png") });
  } finally {
    rmSync(ws, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  }
});
