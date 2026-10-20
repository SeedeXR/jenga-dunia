import { describe, expect, it } from "vitest";
import { adaptLodBias } from "./previewLod";

describe("adaptLodBias", () => {
  it("is off at target 0 and coarsens / refines around the target", () => {
    expect(adaptLodBias(2, 900, 0)).toBe(0);
    expect(adaptLodBias(0, 400, 100)).toBe(1); // too slow: one step coarser
    expect(adaptLodBias(4, 400, 100)).toBe(4); // capped
    expect(adaptLodBias(2, 10, 100)).toBe(1); // well under: one step finer
    expect(adaptLodBias(0, 10, 100)).toBe(0); // never below the default
    expect(adaptLodBias(1, 60, 100)).toBe(1); // in the band: hold
  });
});
