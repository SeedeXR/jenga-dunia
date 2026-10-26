// Languages and the crash reporter: Kiswahili relabels the panels and persists across a reload; an
// uncaught UI error is written as a crash report beside the project's ops log.
import { expect, test } from "@playwright/test";
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, installBridge } from "./bridge";

test.describe("languages and crash reports against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-i18n-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  });

  test("Kiswahili persists; an uncaught error leaves a crash report", async ({ page }) => {
    const core = await connectCore(ws);
    await installBridge(page, core);
    await page.goto("/");
    const proj = path.join(ws, "Lugha.jdproj");
    await page.getByTestId("project-path").fill(proj);
    await page.getByLabel("Extent (m)").fill("1024");
    await page.getByLabel("Resolution (px)").fill("128");
    await page.getByTestId("new-project").click();
    await expect(page.locator("header")).toContainText("World — 128²");

    await page.getByTestId("setting-lang").selectOption("sw");
    await expect(page.getByRole("heading", { name: "Mradi" })).toBeVisible();
    await expect(page.getByRole("heading", { name: "Jua" })).toBeVisible();
    await page.reload();
    await expect(page.getByRole("heading", { name: "Mradi" })).toBeVisible();
    await page.getByTestId("project-path").fill(proj);
    await page.getByTestId("open-project").click();
    await expect(page.locator("header")).toContainText("World — 128²");
    await page.getByTestId("setting-lang").selectOption("en");
    await expect(page.getByRole("heading", { name: "Project" })).toBeVisible();

    // A deliberate uncaught error: the window handler writes a report and logs its path.
    await page.evaluate(() => setTimeout(() => { throw new Error("jd-e2e crash probe"); }, 0));
    await expect(page.getByTestId("log")).toContainText("crash report: ", { timeout: 15_000 });
    const logs = path.join(proj, "logs");
    await expect.poll(() => (existsSync(logs) ? readdirSync(logs).filter((f) => f.startsWith("crash_")).length : 0)).toBeGreaterThan(0);
    const report = JSON.parse(readFileSync(path.join(logs, readdirSync(logs).find((f) => f.startsWith("crash_"))!), "utf8")) as { message: string; source: string };
    expect(report.message).toContain("jd-e2e crash probe");
    expect(report.source).toBe("ui:window");
    await core.close();
  });
});
