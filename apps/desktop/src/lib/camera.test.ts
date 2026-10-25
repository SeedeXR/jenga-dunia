import { describe, expect, it } from "vitest";
import { orbitAt, unwrapThetas, type OrbitKey } from "./camera";

const keys: OrbitKey[] = [
  { theta: 0, phi: 1.0, radius: 4000 },
  { theta: 1, phi: 0.8, radius: 2000 },
  { theta: 2, phi: 1.1, radius: 3000 },
];

describe("flythrough camera path", () => {
  it("passes through every key, at the ends and at the knots", () => {
    const start = orbitAt(keys, 0);
    expect(start.theta).toBeCloseTo(0, 9);
    expect(start.phi).toBeCloseTo(1.0, 9);
    expect(start.radius).toBeCloseTo(4000, 6);
    const mid = orbitAt(keys, 0.5);
    expect(mid.theta).toBeCloseTo(1, 9);
    expect(mid.phi).toBeCloseTo(0.8, 9);
    expect(mid.radius).toBeCloseTo(2000, 6);
    const end = orbitAt(keys, 1);
    expect(end.theta).toBeCloseTo(2, 9);
    expect(end.radius).toBeCloseTo(3000, 6);
  });

  it("is smooth through a key: the slopes either side match", () => {
    const h = 1e-4;
    const before = (orbitAt(keys, 0.5).phi - orbitAt(keys, 0.5 - h).phi) / h;
    const after = (orbitAt(keys, 0.5 + h).phi - orbitAt(keys, 0.5).phi) / h;
    expect(Math.abs(before - after)).toBeLessThan(1e-2);
  });

  it("crosses due north the short way and zooms evenly", () => {
    const wrapped = unwrapThetas([{ theta: 6.1, phi: 1, radius: 1000 }, { theta: 0.1, phi: 1, radius: 1000 }]);
    expect(wrapped[1]!.theta).toBeCloseTo(0.1 + 2 * Math.PI, 9);
    const zoom = orbitAt([{ theta: 0, phi: 1, radius: 1000 }, { theta: 0, phi: 1, radius: 4000 }], 0.5);
    expect(zoom.radius).toBeCloseTo(2000, 6); // the geometric mean
  });
});
