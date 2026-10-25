import { describe, expect, it } from "vitest";
import { evalExpr, num } from "./expr";

describe("expressions in number fields", () => {
  it("evaluates arithmetic with precedence, parentheses, powers and functions", () => {
    expect(evalExpr("512*2")).toBe(1024);
    expect(evalExpr("1 + 2 * 3")).toBe(7);
    expect(evalExpr("(1 + 2) * 3")).toBe(9);
    expect(evalExpr("2^10")).toBe(1024);
    expect(evalExpr("2^3^2")).toBe(512);
    expect(evalExpr("-4 + 10 % 3")).toBe(-3);
    expect(evalExpr("sqrt(16) + max(1, 5, 3)")).toBe(9);
    expect(evalExpr("pi")).toBeCloseTo(Math.PI);
    expect(evalExpr("1.5e3 / 3")).toBe(500);
  });
  it("refuses what is not an expression", () => {
    expect(evalExpr("")).toBeNull();
    expect(evalExpr("abc")).toBeNull();
    expect(evalExpr("1 +")).toBeNull();
    expect(evalExpr("1/0")).toBeNull();
    expect(evalExpr("nope(2)")).toBeNull();
    expect(evalExpr("2 3")).toBeNull();
  });
  it("num keeps plain numbers exact and falls back to NaN", () => {
    expect(num("42")).toBe(42);
    expect(num(" 4*2+1 ")).toBe(9);
    expect(Number.isNaN(num("x"))).toBe(true);
  });
});
