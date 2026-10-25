import { expect, test } from "@playwright/test";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { connectCore, installBridge } from "./bridge";

/** A tiny MP4 that states only where and when it was shot (Android ©xyz, mvhd). */
function clip(): Buffer {
  const bx = (ty: string, payload: Buffer) => Buffer.concat([Buffer.from(new Uint8Array(new Uint32Array([payload.length + 8]).buffer).reverse()), Buffer.from(ty, "latin1"), payload]);
  const u32 = (n: number) => Buffer.from(new Uint8Array(new Uint32Array([n]).buffer).reverse());
  const created = 3_864_880_800; // 2026-06-21T10:00:00Z in seconds since 1904
  const mvhd = Buffer.concat([Buffer.alloc(4), u32(created), u32(created), u32(1000), u32(5000), Buffer.alloc(80)]);
  const xyz = Buffer.concat([Buffer.from([0, 18, 0x15, 0xc7]), Buffer.from("-24.7500+015.3500/", "latin1")]);
  return Buffer.concat([bx("ftyp", Buffer.from("isom\0\0\0\0isom", "latin1")), bx("moov", Buffer.concat([bx("mvhd", mvhd), bx("udta", bx("\xa9xyz", xyz))]))]);
}

test.describe("reference image against the real core", () => {
  let ws: string;
  test.beforeEach(() => {
    ws = mkdtempSync(path.join(tmpdir(), "jd-e2e-ref-"));
  });
  test.afterEach(() => {
    rmSync(ws, { recursive: true, force: true });
  });

  test("reading a reference shows where, when and the sun; Use sun sets the viewport sky", async ({ page }) => {
    const core = await connectCore(ws);
    await installBridge(page, core);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto("/");
    await page.getByTestId("project-path").fill(path.join(ws, "Ref.jdproj"));
    await page.getByLabel("Extent (m)").fill("1024");
    await page.getByLabel("Resolution (px)").fill("128");
    await page.getByTestId("new-project").click();
    await expect(page.locator("header")).toContainText("World — 128²");
    const file = path.join(ws, "dunes.mp4");
    writeFileSync(file, clip());
    await page.getByTestId("ref-path").fill(file);
    await page.getByTestId("ref-read").click();
    await expect(page.getByTestId("log")).toContainText("media.metadata: ok");
    const info = page.getByTestId("ref-info");
    await expect(info).toContainText("video");
    await expect(info).toContainText("2026-06-21T10:00:00Z");
    await expect(info).toContainText("-24.7500, 15.3500");
    await expect(info).toContainText(/\d+° from north, \d+° up/); // the Namib, mid-morning in June
    await expect(page.getByTestId("ref-terrain")).toBeEnabled();
    await page.getByTestId("ref-use-sun").click();
    const az = Number(await page.getByTestId("sun-azimuth").inputValue());
    const el = Number(await page.getByTestId("sun-elevation").inputValue());
    expect(az).toBeGreaterThan(0);
    expect(az).toBeLessThan(90); // a north-eastern morning sun in the southern winter
    expect(el).toBeGreaterThan(20);
    expect(el).toBeLessThan(60);
    expect(errors, "no uncaught page errors").toEqual([]);
    await core.close();
  });
});
