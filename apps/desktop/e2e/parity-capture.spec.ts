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
  } finally {
    rmSync(ws, { recursive: true, force: true });
  }
});
