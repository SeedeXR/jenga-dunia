import { useLayoutEffect, useRef, type InputHTMLAttributes } from "react";

type Props = Omit<InputHTMLAttributes<HTMLInputElement>, "type"> & {
  /** Double-click resets the slider (Lightroom: double-click a slider to reset it). */
  onReset?: () => void;
  /** Where the fill starts, for ranges centred on a default (e.g. exposure 1); default: min. */
  fillFrom?: number;
};

/**
 * The Lightroom-style slider (docs/research/research-log/2026-10-26-lightroom-slider.md): the native
 * range input — so keyboard, accessibility and tests behave exactly as before — drawn as a 2 px
 * track with a fill and a ring handle (`.jd-range` in index.css). Studio's egui slider draws the same.
 */
export function Range({ onReset, fillFrom, className, onInput, onDoubleClick, ...rest }: Props) {
  const ref = useRef<HTMLInputElement>(null);
  const paint = () => {
    const el = ref.current;
    if (!el) return;
    const min = Number(el.min || 0);
    const max = Number(el.max || 100);
    const at = (x: number) => ((Math.min(max, Math.max(min, x)) - min) / (max - min || 1)) * 100;
    const [a, b] = [at(fillFrom ?? min), at(Number(el.value))].sort((x, y) => x - y);
    el.style.setProperty("--fill-a", `${a}%`);
    el.style.setProperty("--fill-b", `${b}%`);
  };
  // Every render: a controlled value may have changed from outside (a reset, a loaded project).
  useLayoutEffect(paint);
  return (
    <input
      ref={ref}
      type="range"
      className={`jd-range ${className ?? ""}`}
      onInput={(e) => {
        paint();
        onInput?.(e);
      }}
      onDoubleClick={(e) => {
        onReset?.();
        onDoubleClick?.(e);
      }}
      {...rest}
    />
  );
}
