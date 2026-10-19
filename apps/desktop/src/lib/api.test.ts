import { describe, expect, it } from "vitest";
import { PREVIEW_HEADER_BYTES, decodePreview } from "./api";

function encode(width: number, height: number, min: number, max: number, data: number[]): ArrayBuffer {
  const buf = new ArrayBuffer(PREVIEW_HEADER_BYTES + data.length * 2);
  const v = new DataView(buf);
  v.setUint32(0, width, true);
  v.setUint32(4, height, true);
  v.setFloat32(8, min, true);
  v.setFloat32(12, max, true);
  data.forEach((d, i) => v.setUint16(PREVIEW_HEADER_BYTES + i * 2, d, true));
  return buf;
}

describe("decodePreview", () => {
  it("decodes the jd-terrain wire format", () => {
    const p = decodePreview(encode(3, 2, -5, 120, [0, 1, 65535, 7, 8, 9]));
    expect(p.width).toBe(3);
    expect(p.height).toBe(2);
    expect(p.min).toBeCloseTo(-5);
    expect(p.max).toBeCloseTo(120);
    expect(Array.from(p.data)).toEqual([0, 1, 65535, 7, 8, 9]);
  });
  it("rejects short and truncated buffers", () => {
    expect(() => decodePreview(new ArrayBuffer(4))).toThrow(/short/);
    expect(() => decodePreview(encode(4, 4, 0, 1, [1, 2]))).toThrow(/truncated/);
  });
});
