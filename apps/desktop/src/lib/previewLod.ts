/** Live-preview LOD policy: a target evaluation time moves the preview up or down the LOD ladder.
 *  `bias` is added to the project's preview LOD (0 = the ≤ 512² default). Over target → coarser;
 *  comfortably under (a quarter of the target) → finer, never below the default. 0 = off. */
export function adaptLodBias(bias: number, durationMs: number, targetMs: number, maxBias = 4): number {
  if (!(targetMs > 0)) return 0;
  if (durationMs > targetMs) return Math.min(maxBias, bias + 1);
  if (durationMs * 4 < targetMs) return Math.max(0, bias - 1);
  return bias;
}
