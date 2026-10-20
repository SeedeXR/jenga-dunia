import { describe, expect, it } from "vitest";
import * as THREE from "three";
import { buildSeaGeometry, waterColour } from "./TerrainView";
import type { Preview } from "../lib/api";

// A 4×4 preview sloping west → east from 0 to 300 m (u16 quantised over min..max).
function ramp(): Preview {
  const width = 4;
  const height = 4;
  const data = new Uint16Array(width * height);
  for (let j = 0; j < height; j++) for (let i = 0; i < width; i++) data[j * width + i] = Math.round((i / 3) * 65535);
  return { width, height, min: 0, max: 300, data };
}

describe("buildSeaGeometry", () => {
  it("is transparent over land, foamy at the shore and opaque out deep", () => {
    const geo = buildSeaGeometry(ramp(), [300, 300], 110);
    const color = geo.getAttribute("color");
    expect(color.itemSize).toBe(4);
    // Column 0 (height 0) is 110 m under water: deep, mostly opaque, blue rather than red.
    expect(color.getW(0)).toBeGreaterThan(0.75);
    expect(color.getZ(0)).toBeGreaterThan(color.getX(0));
    // Column 3 (height 300) is land: fully transparent.
    expect(color.getW(3)).toBe(0);
    // Column 1 (height 100, depth 10 m) is shallower than column 0: less opaque, and the sandy
    // bottom still shows in green (red is absorbed within a few metres).
    expect(color.getW(1)).toBeLessThan(color.getW(0));
    expect(color.getY(1)).toBeGreaterThan(color.getY(0));
    // A sea level just above a vertex puts that vertex in the foam band: pale and opaque.
    const foam = buildSeaGeometry(ramp(), [300, 300], 100.5).getAttribute("color");
    // Colours are linear-light (three.js converts the hex constants), so foam red ≈ 0.63, shallow ≈ 0.08.
    expect(foam.getX(1)).toBeGreaterThan(0.5);
    expect(foam.getW(1)).toBeGreaterThan(0.8); // 0.5 + 0.45 · (1 − 0.5 m / 3 m) = 0.875
  });

  it("clarity follows Beer–Lambert: murky water hides the bottom sooner and takes the tint", () => {
    const c = new THREE.Color();
    const clear = waterColour(10, { visibilityM: 30, tint: [0.1, 0.35, 0.4] }, c);
    const clearRgb = [c.r, c.g, c.b];
    const murky = waterColour(10, { visibilityM: 3, tint: [0.35, 0.3, 0.15] }, c);
    // Ten metres of crystal-clear water still shows most of the bottom; the same depth of murky water does not.
    expect(clear.clarity).toBeGreaterThan(0.4);
    expect(murky.clarity).toBeLessThan(0.25);
    expect(murky.alpha).toBeGreaterThan(clear.alpha);
    // Murky water is the silt tint; clear water keeps the sandy bottom's red/green and loses red first.
    expect(Math.abs(c.r - 0.35)).toBeLessThan(0.05);
    expect(clearRgb[0]!).toBeLessThan(0.62);
    expect(clearRgb[2]!).toBeGreaterThan(0.37);
    // Deep clear water converges on the tint too.
    const deep = waterColour(200, { visibilityM: 30, tint: [0.1, 0.35, 0.4] }, c);
    expect(deep.clarity).toBeLessThan(0.2); // blue still carries at 200 m in clear water
    expect(Math.abs(c.b - 0.4)).toBeLessThan(0.02);
  });
});
