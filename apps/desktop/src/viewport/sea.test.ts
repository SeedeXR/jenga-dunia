import { describe, expect, it } from "vitest";
import * as THREE from "three";
import { buildFrame, buildSeaGeometry, buildShoreline, gerstner, waterColour } from "./TerrainView";
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

describe("gerstner", () => {
  const east = { amplitude_m: 1, wavelength_m: 100, direction_deg: 90, speed_mps: 12.5, phase: 0 };
  it("lifts a crest a quarter wavelength downwind and pushes the surface horizontally at the node", () => {
    // Travelling east: x = 25 m is a quarter wavelength into the wave (φ = π/2) → full height, no slide.
    const crest = gerstner([east], 25, 0, 0, [0, 0, 0]);
    expect(crest[1]).toBeCloseTo(1);
    expect(crest[0]).toBeCloseTo(0);
    // At the node (φ = 0) the orbit is all horizontal, bounded by the steepness Q·A ≤ A.
    const node = gerstner([east], 0, 0, 0, [0, 0, 0]);
    expect(node[1]).toBeCloseTo(0);
    expect(node[0]).toBeGreaterThan(0);
    expect(node[0]).toBeLessThanOrEqual(1);
    // One period later (L / c = 8 s) the surface is back where it was.
    expect(gerstner([east], 25, 0, 8, [0, 0, 0])[1]).toBeCloseTo(1);
    // A northbound wave moves vertices along −z, never x.
    const north = gerstner([{ ...east, direction_deg: 0 }], 0, 0, 0, [0, 0, 0]);
    expect(north[0]).toBeCloseTo(0);
    expect(north[2]).toBeLessThan(0);
  });
  it("widens the shoreline foam to the surf zone when the sea breaks", () => {
    // Column 1 (depth 10 m) is deep blue-green water at 110 m without waves, foam when waves break above 50 m.
    const calm = buildSeaGeometry(ramp(), [300, 300], 110).getAttribute("color");
    const surf = buildSeaGeometry(ramp(), [300, 300], 110, { visibilityM: 15, tint: [0.1, 0.35, 0.4], breakingDepthM: 50 }).getAttribute("color");
    expect(surf.getX(1)).toBeGreaterThan(calm.getX(1) + 0.3);
    expect(surf.getW(1)).toBeGreaterThan(calm.getW(1));
  });
});

describe("shoreline and frame", () => {
  it("the shoreline runs north–south where the west→east ramp crosses the sea", () => {
    // The ramp is 0, 100, 200, 300 m across the columns; a 150 m sea crosses between columns 1 and 2.
    const seg = buildShoreline(ramp(), [300, 300], 150);
    expect(seg.length).toBe(3 * 6); // three cells down the crossing column, one segment each
    for (let k = 0; k < seg.length; k += 3) expect(seg[k]).toBeCloseTo(0, 5); // x = 0: half-way across the 300 m world
    expect(buildShoreline(ramp(), [300, 300], 400).length).toBe(0); // above every height: no shore
  });

  it("the frame walks the boundary once: walls down to the base, a rim on top and bottom", () => {
    const { walls, rim } = buildFrame(ramp(), [300, 300], -50);
    const ring = 4 * (4 - 1); // boundary vertices of a 4×4 grid
    expect(walls.getAttribute("position").count).toBe(ring * 6);
    expect(rim.length).toBe(ring * 4 * 3);
    const ys = Array.from({ length: rim.length / 3 }, (_, k) => rim[k * 3 + 1]!);
    expect(Math.min(...ys)).toBe(-50);
    expect(Math.max(...ys)).toBeCloseTo(300, 3);
  });
});
