// Camera paths for flythroughs: a Catmull-Rom curve through orbit keys (theta, phi, radius), so the
// camera eases through every key instead of turning sharply at it. Theta is unwrapped first, so a
// path across due north never spins the long way round; the radius is interpolated in log space,
// so a zoom feels even. Pure functions — the viewport only applies the result.

export interface OrbitKey {
  theta: number;
  phi: number;
  radius: number;
}

/** Theta values made continuous: each differs from the previous by at most π. */
export function unwrapThetas(keys: OrbitKey[]): OrbitKey[] {
  const out: OrbitKey[] = [];
  for (const k of keys) {
    const prev = out[out.length - 1];
    if (!prev) {
      out.push({ ...k });
      continue;
    }
    let t = k.theta;
    while (t - prev.theta > Math.PI) t -= 2 * Math.PI;
    while (t - prev.theta < -Math.PI) t += 2 * Math.PI;
    out.push({ ...k, theta: t });
  }
  return out;
}

function catmullRom(p0: number, p1: number, p2: number, p3: number, u: number): number {
  const u2 = u * u;
  const u3 = u2 * u;
  return 0.5 * (2 * p1 + (-p0 + p2) * u + (2 * p0 - 5 * p1 + 4 * p2 - p3) * u2 + (-p0 + 3 * p1 - 3 * p2 + p3) * u3);
}

/** The orbit at path parameter `t` in 0..1 through every key (uniform Catmull-Rom, ends clamped). */
export function orbitAt(keys: OrbitKey[], t: number): OrbitKey {
  if (keys.length === 0) throw new Error("no keys");
  if (keys.length === 1) return { ...keys[0]! };
  const k = unwrapThetas(keys);
  const n = k.length - 1;
  const x = Math.min(Math.max(t, 0), 1) * n;
  const seg = Math.min(n - 1, Math.floor(x));
  const u = x - seg;
  const at = (i: number) => k[Math.min(n, Math.max(0, i))]!;
  const [a, b, c, d] = [at(seg - 1), at(seg), at(seg + 1), at(seg + 2)];
  return {
    theta: catmullRom(a.theta, b.theta, c.theta, d.theta, u),
    phi: catmullRom(a.phi, b.phi, c.phi, d.phi, u),
    radius: Math.exp(catmullRom(Math.log(a.radius), Math.log(b.radius), Math.log(c.radius), Math.log(d.radius), u)),
  };
}
