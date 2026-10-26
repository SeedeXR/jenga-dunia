import { fireEvent, render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { Range } from "./Range";

describe("Range", () => {
  it("is a native range input whose fill follows the value", () => {
    const { getByTestId, rerender } = render(<Range min={0} max={10} value={5} onChange={() => {}} data-testid="r" />);
    const el = getByTestId("r") as HTMLInputElement;
    expect(el.type).toBe("range");
    expect(el.className).toContain("jd-range");
    expect(el.style.getPropertyValue("--fill-a")).toBe("0%");
    expect(el.style.getPropertyValue("--fill-b")).toBe("50%");
    rerender(<Range min={0} max={10} value={10} onChange={() => {}} data-testid="r" />);
    expect(el.style.getPropertyValue("--fill-b")).toBe("100%");
  });

  it("fills from fillFrom for centred ranges and resets on double-click", () => {
    const reset = vi.fn();
    const { getByTestId } = render(<Range min={0} max={4} value={0.5} fillFrom={1} onReset={reset} onChange={() => {}} data-testid="r" />);
    const el = getByTestId("r") as HTMLInputElement;
    expect(el.style.getPropertyValue("--fill-a")).toBe("12.5%");
    expect(el.style.getPropertyValue("--fill-b")).toBe("25%");
    fireEvent.doubleClick(el);
    expect(reset).toHaveBeenCalledTimes(1);
  });
});
