import { describe, expect, it } from "vitest";
import { buildSeaGeometry } from "./TerrainView";
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
    const geo = buildSeaGeometry(ramp(), [300, 300], 150);
    const color = geo.getAttribute("color");
    expect(color.itemSize).toBe(4);
    // Column 0 (height 0) is 150 m under water: deep, mostly opaque, navy rather than turquoise.
    expect(color.getW(0)).toBeGreaterThan(0.75);
    expect(color.getZ(0)).toBeGreaterThan(color.getX(0));
    // Column 3 (height 300) is land: fully transparent.
    expect(color.getW(3)).toBe(0);
    // Column 1 (height 100, depth 50 m) is shallower than column 0: lighter and less opaque.
    expect(color.getW(1)).toBeLessThan(color.getW(0));
    expect(color.getX(1)).toBeGreaterThan(color.getX(0));
    // A sea level just above a vertex puts that vertex in the foam band: pale and opaque.
    const foam = buildSeaGeometry(ramp(), [300, 300], 100.5).getAttribute("color");
    // Colours are linear-light (three.js converts the hex constants), so foam red ≈ 0.63, shallow ≈ 0.08.
    expect(foam.getX(1)).toBeGreaterThan(0.5);
    expect(foam.getW(1)).toBeGreaterThan(0.8); // 0.5 + 0.45 · (1 − 0.5 m / 3 m) = 0.875
  });
});
