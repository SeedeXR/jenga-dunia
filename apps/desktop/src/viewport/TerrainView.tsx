// Terrain viewport: three.js WebGPURenderer with automatic WebGL 2 fallback (ADR-0003).
import type { OceanWavesResult, WaveComponent } from "../lib/ops.gen";
// The mesh is a regular grid displaced on the CPU from the u16 preview; colours follow height.
import { useEffect, useRef } from "react";
import * as THREE from "three";
import { WebGPURenderer } from "three/webgpu";
import type { Preview } from "../lib/api";

export interface ViewportInfo {
  backend: "webgpu" | "webgl2" | "none";
  triangles: number;
  /** Sea plane height in metres when one is drawn. */
  seaM?: number;
  /** Sun direction currently lighting the scene, degrees. */
  sun?: { azimuthDeg: number; elevationDeg: number };
  /** Scatter instances drawn, when any. */
  instances?: number;
  /** Baked material maps applied (texture size), when any. */
  pbrPx?: [number, number];
  /** Wind sea being animated: significant height and peak wavelength, metres. */
  waves?: { heightM: number; lengthM: number };
  /** Physical sky texture on the dome (equirectangular preview size), when one is applied. */
  skyPx?: [number, number];
  /** Horizon skirt resolution per side, when one is drawn. */
  horizonPx?: number;
}

interface Props {
  preview: Preview | null;
  /** Colour vertices from this layer's preview instead of the height ramp. */
  overlay?: Preview | null;
  extentM: [number, number] | null;
  onInfo?: (info: ViewportInfo) => void;
  /** Sculpt mode: pointer drags paint a stroke on the terrain instead of orbiting. Points are
   *  reported in layer metres (u east from the west edge, v north from the south edge = row 0). */
  sculpt?: { enabled: boolean; radiusM: number; onStroke: (points: [number, number][]) => void };
  /** Draw a translucent water plane at this height (metres); null hides it. */
  seaLevelM?: number | null;
  /** Water clarity and tint (from the sea-level node); defaults to open-ocean water. */
  water?: WaterLook | null;
  /** Wind sea (`ocean.waves` for the sea-level node's wind): the sea surface is animated as a Gerstner sum of its components. */
  waves?: OceanWavesResult | null;
  /** Physical sky (`sky.preview`, equirectangular, row 0 = zenith, column 0 = north) drawn on the dome; null keeps the gradient. */
  sky?: { bitmap: ImageBitmap; px: [number, number] } | null;
  /** The noise continued `scale` × beyond the border: a coarse skirt drawn under the terrain with a hole for the world. */
  horizon?: { preview: Preview; scale: number } | null;
  /** Sun direction: azimuth clockwise from north (0 = light from the north), elevation above the horizon. */
  sun?: { azimuthDeg: number; elevationDeg: number };
  /** Baked albedo/normal/roughness maps for the shown layer; textured when set and no overlay is active. */
  maps?: { albedo: ImageBitmap; normal: ImageBitmap; roughness: ImageBitmap; px: [number, number] } | null;
  /** Scatter instances (layer metres: u east, v north from the south edge, z height, r 0..1) drawn as
   *  `sizeM`-tall cones — a preview marker, the exported GLB carries 1 m proxies. */
  instances?: { points: { u: number; v: number; z: number; r: number }[]; sizeM: number } | null;
}

function isWebGPU(r: WebGPURenderer): boolean {
  // three.js marks the backend instance; the type declarations do not expose the flag.
  return Boolean((r.backend as { isWebGPUBackend?: boolean }).isWebGPUBackend);
}

// Hypsometric ramp (lowland green → ochre → bare rock → snow) by normalised height, blended
// towards rock grey on steep slopes. Readable at a glance like World Creator's default look,
// while staying a preview: materials come from the material graph later.
// Precomputed once (256 entries) so geometry builds allocate nothing per vertex.
const STOPS: [number, THREE.Color][] = [
  [0.0, new THREE.Color(0.16, 0.3, 0.2)],
  [0.25, new THREE.Color(0.36, 0.48, 0.24)],
  [0.5, new THREE.Color(0.62, 0.52, 0.34)],
  [0.75, new THREE.Color(0.52, 0.5, 0.48)],
  [1.0, new THREE.Color(0.95, 0.95, 0.97)],
];
const ROCK = new THREE.Color(0.42, 0.4, 0.38);
const RAMP: Float32Array = (() => {
  const out = new Float32Array(256 * 3);
  const c = new THREE.Color();
  for (let i = 0; i < 256; i++) {
    const t = i / 255;
    let k = 0;
    while (k < STOPS.length - 2 && t > (STOPS[k + 1]?.[0] ?? 1)) k++;
    const [t0, c0] = STOPS[k] ?? STOPS[0]!;
    const [t1, c1] = STOPS[k + 1] ?? STOPS[STOPS.length - 1]!;
    c.copy(c0).lerp(c1, (t - t0) / Math.max(1e-6, t1 - t0));
    out[i * 3] = c.r;
    out[i * 3 + 1] = c.g;
    out[i * 3 + 2] = c.b;
  }
  return out;
})();

/** Vertex colour from an overlay layer: class maps (small integers) get a palette, masks a
 *  dark→accent gradient. The overlay may be a different size than the height preview. */
function overlayColor(o: Preview, u: number, v: number, out: THREE.Color): void {
  const x = Math.min(o.width - 1, Math.max(0, Math.round(u * (o.width - 1))));
  const y = Math.min(o.height - 1, Math.max(0, Math.round(v * (o.height - 1))));
  const q = o.data[y * o.width + x] ?? 0;
  const value = o.min + (q / 65535) * (o.max - o.min);
  const classes = o.max <= 32 && o.min >= 0 && Number.isInteger(o.max) && Number.isInteger(o.min);
  if (classes) {
    const id = Math.round(value);
    if (id <= 0) out.setRGB(0.12, 0.12, 0.14);
    else out.setHSL(((id - 1) * 0.618034) % 1, 0.55, 0.45); // golden-angle palette
  } else {
    const t = o.max > o.min ? (value - o.min) / (o.max - o.min) : 0;
    out.setRGB(0.1 + 0.15 * t, 0.12 + 0.6 * t, 0.16 + 0.75 * t);
  }
}

export function buildTerrainGeometry(p: Preview, extent: [number, number], overlay: Preview | null = null): THREE.BufferGeometry {
  const { width, height, min, max, data } = p;
  const geo = new THREE.PlaneGeometry(extent[0], extent[1], width - 1, height - 1);
  geo.rotateX(-Math.PI / 2); // plane in XZ, +Y up (glTF frame)
  const pos = geo.attributes.position as THREE.BufferAttribute;
  const colors = new Float32Array(pos.count * 3);
  const tmp = new THREE.Color();
  const range = max - min || 1;
  for (let j = 0; j < height; j++) {
    // PlaneGeometry rows run from +Z (front) to -Z; the preview's row 0 is south (+Z after rotation).
    for (let i = 0; i < width; i++) {
      const vi = j * width + i;
      const q = data[(height - 1 - j) * width + i] ?? 0;
      pos.setY(vi, min + (q / 65535) * range);
      if (overlay) {
        overlayColor(overlay, i / Math.max(1, width - 1), (height - 1 - j) / Math.max(1, height - 1), tmp);
        colors[vi * 3] = tmp.r;
        colors[vi * 3 + 1] = tmp.g;
        colors[vi * 3 + 2] = tmp.b;
        continue;
      }
      const r = (q >> 8) * 3;
      colors[vi * 3] = RAMP[r] ?? 0;
      colors[vi * 3 + 1] = RAMP[r + 1] ?? 0;
      colors[vi * 3 + 2] = RAMP[r + 2] ?? 0;
    }
  }
  pos.needsUpdate = true;
  geo.computeVertexNormals();
  // Steep faces read as rock regardless of altitude: blend by (1 − normal.y), i.e. slope.
  const nrm = geo.attributes.normal as THREE.BufferAttribute;
  for (let vi = 0; vi < pos.count && !overlay; vi++) {
    const slope = Math.min(1, Math.max(0, (1 - nrm.getY(vi)) * 1.4));
    const [r, g, b] = [colors[vi * 3] ?? 0, colors[vi * 3 + 1] ?? 0, colors[vi * 3 + 2] ?? 0];
    colors[vi * 3] = r + (ROCK.r - r) * slope;
    colors[vi * 3 + 1] = g + (ROCK.g - g) * slope;
    colors[vi * 3 + 2] = b + (ROCK.b - b) * slope;
  }
  geo.setAttribute("color", new THREE.BufferAttribute(colors, 3));
  return geo;
}

/** Water clarity settings mirrored from the sea-level node. */
export interface WaterLook {
  /** Depth at which the bottom fades out (m): ~30 crystal clear, ~15 ocean, ~3 murky estuary. */
  visibilityM: number;
  /** Colour of the suspended matter that scatters back (linear RGB 0..1). */
  tint: [number, number, number];
  /** Depth (m) above which the wind sea breaks (H/d = 0.78): foam covers the surf zone. Default: none. */
  breakingDepthM?: number;
}

/** Gerstner (trochoidal) displacement of the surface point (`x` east, `z` south in the terrain frame)
 *  at `tS` seconds: the sum of the components' circular orbits — crests sharpen, troughs flatten.
 *  Directions are degrees clockwise from north; steepness is shared so the sum never cusps. */
export function gerstner(components: WaveComponent[], x: number, z: number, tS: number, out: [number, number, number]): [number, number, number] {
  out[0] = out[1] = out[2] = 0;
  const n = components.length;
  for (const c of components) {
    const k = (2 * Math.PI) / c.wavelength_m;
    const rad = (c.direction_deg * Math.PI) / 180;
    const dx = Math.sin(rad);
    const dz = -Math.cos(rad); // north is −z
    const phi = k * (dx * x + dz * z) - k * c.speed_mps * tS + c.phase;
    const q = Math.min(1, 0.5 / (k * c.amplitude_m * n)); // GPU Gems 1 ch. 1: Qᵢ = Q / (kᵢ Aᵢ N)
    const cos = Math.cos(phi);
    out[0] += q * c.amplitude_m * dx * cos;
    out[2] += q * c.amplitude_m * dz * cos;
    out[1] += c.amplitude_m * Math.sin(phi);
  }
  return out;
}

/** Pure-water absorption per channel in m⁻¹ (Pope & Fry 1997, rounded): red dies first, blue last. */
const PURE_WATER_ABSORPTION: [number, number, number] = [0.4, 0.07, 0.005];
const DEFAULT_WATER: WaterLook = { visibilityM: 15, tint: [0.1, 0.35, 0.4] };

/** What a sea floor of a given depth looks like through the water: the sandy bottom seen through
 *  Beer–Lambert transmittance per channel (absorption scaled by 15 m / visibility, light travels
 *  down and back up), plus the tint scattered back in proportion to what was absorbed. */
export function waterColour(depthM: number, look: WaterLook, out: THREE.Color): { alpha: number; clarity: number } {
  const k = 15 / Math.max(0.5, look.visibilityM);
  const bottom = [0.62, 0.55, 0.38]; // wet sand, linear
  const rgb = [0, 0, 0];
  let seen = 0;
  for (let c = 0; c < 3; c++) {
    const t = Math.exp(-PURE_WATER_ABSORPTION[c]! * k * depthM * 2);
    rgb[c] = bottom[c]! * t + (look.tint[c] ?? 0) * (1 - t);
    seen += t;
  }
  const clarity = seen / 3;
  out.setRGB(rgb[0]!, rgb[1]!, rgb[2]!);
  // Murkier or deeper water hides more: opacity grows as the bottom disappears.
  return { alpha: 0.35 + 0.55 * (1 - clarity), clarity };
}

/** Water surface at `seaM`: a flat grid whose vertex colour and alpha follow the depth of the terrain
 *  below it and the water's clarity — the sandy bottom shows through clear shallows, the tint takes
 *  over as depth or turbidity grows; a pale foam band where the depth is within `foamM` of the
 *  shore, fully transparent over land. */
export function buildSeaGeometry(p: Preview, extent: [number, number], seaM: number, look: WaterLook = DEFAULT_WATER): THREE.BufferGeometry {
  const { width, height, min, max, data } = p;
  const geo = new THREE.PlaneGeometry(extent[0], extent[1], width - 1, height - 1);
  geo.rotateX(-Math.PI / 2);
  const pos = geo.attributes.position as THREE.BufferAttribute;
  const colors = new Float32Array(pos.count * 4);
  const range = max - min || 1;
  // Foam: a thin shoreline band, widened to the surf zone when a wind sea breaks on the shelf.
  const foamM = Math.max(0.5, range * 0.01, look.breakingDepthM ?? 0);
  const foam = new THREE.Color(0xdff3f5);
  const c = new THREE.Color();
  for (let j = 0; j < height; j++) {
    for (let i = 0; i < width; i++) {
      const vi = j * width + i;
      const q = data[(height - 1 - j) * width + i] ?? 0;
      const depth = seaM - (min + (q / 65535) * range);
      let alpha = 0;
      if (depth > 0) {
        alpha = waterColour(depth, look, c).alpha;
        if (depth < foamM) {
          const f = 1 - depth / foamM;
          c.lerp(foam, f);
          alpha = Math.max(alpha, 0.5 + 0.45 * f);
        }
      } else {
        waterColour(0, look, c);
      }
      colors[vi * 4] = c.r;
      colors[vi * 4 + 1] = c.g;
      colors[vi * 4 + 2] = c.b;
      colors[vi * 4 + 3] = alpha;
    }
  }
  geo.setAttribute("color", new THREE.BufferAttribute(colors, 4));
  return geo;
}

interface SceneState {
  renderer: WebGPURenderer;
  scene: THREE.Scene;
  camera: THREE.PerspectiveCamera;
  mesh: THREE.Mesh<THREE.BufferGeometry, THREE.MeshStandardMaterial> | null;
  /** Brush ring shown under the pointer in sculpt mode. */
  brush: THREE.Mesh<THREE.RingGeometry, THREE.MeshBasicMaterial>;
  frame: number;
  backend: ViewportInfo["backend"];
  triangles: number;
  sea: THREE.Mesh<THREE.BufferGeometry, THREE.MeshStandardMaterial> | null;
  /** Rest positions of the sea vertices and the wind sea displacing them each frame. */
  seaWaves: { base: Float32Array; components: WaveComponent[]; heightM: number; lengthM: number } | null;
  sun: THREE.DirectionalLight;
  hemi: THREE.HemisphereLight;
  /** Gradient sky dome (vertex colours, inside faces). */
  sky: THREE.Mesh<THREE.SphereGeometry, THREE.MeshBasicMaterial>;
  sunDeg: { azimuthDeg: number; elevationDeg: number };
  orbit: { theta: number; phi: number; radius: number; minR: number; maxR: number };
  instances: THREE.InstancedMesh<THREE.ConeGeometry, THREE.MeshStandardMaterial> | null;
  /** Size of the baked maps currently on the terrain material. */
  pbrPx: [number, number] | null;
  /** Size of the sky texture on the dome, when one is applied. */
  skyPx: [number, number] | null;
  /** Horizon skirt mesh, when drawn. */
  skirt: THREE.Mesh<THREE.BufferGeometry, THREE.MeshStandardMaterial> | null;
}

function placeCamera(s: SceneState) {
  const { theta, phi, radius } = s.orbit;
  s.camera.position.set(radius * Math.sin(phi) * Math.sin(theta), radius * Math.cos(phi), radius * Math.sin(phi) * Math.cos(theta));
  s.camera.lookAt(0, 0, 0);
}

/** One report shape for every effect, so a later report never drops a field an earlier one set. */
function infoOf(s: SceneState): ViewportInfo {
  return {
    backend: s.backend,
    triangles: s.triangles,
    ...(s.sea ? { seaM: s.sea.position.y } : {}),
    ...(s.sea && s.seaWaves ? { waves: { heightM: s.seaWaves.heightM, lengthM: s.seaWaves.lengthM } } : {}),
    sun: s.sunDeg,
    ...(s.instances ? { instances: s.instances.count } : {}),
    ...(s.pbrPx ? { pbrPx: s.pbrPx } : {}),
    ...(s.skyPx ? { skyPx: s.skyPx } : {}),
    ...(s.skirt ? { horizonPx: Math.round(Math.sqrt((s.skirt.geometry.attributes.position?.count ?? 0))) } : {}),
  };
}

/** Move the sea vertices along their Gerstner orbits for this frame (CPU: the sea grid is the
 *  preview's size, a few thousand vertices). ponytail: skip above 70k vertices — a TSL position
 *  node is the upgrade if the sea ever needs the full-resolution preview. */
function animateSea(s: SceneState, tS: number): void {
  const w = s.seaWaves;
  if (!s.sea || !w || w.components.length === 0) return;
  const pos = s.sea.geometry.attributes.position as THREE.BufferAttribute;
  if (pos.count > 70_000) return;
  const arr = pos.array as Float32Array;
  const d: [number, number, number] = [0, 0, 0];
  for (let i = 0; i < pos.count; i++) {
    const x = w.base[i * 3]!;
    const z = w.base[i * 3 + 2]!;
    gerstner(w.components, x, z, tS, d);
    arr[i * 3] = x + d[0];
    arr[i * 3 + 1] = w.base[i * 3 + 1]! + d[1];
    arr[i * 3 + 2] = z + d[2];
  }
  pos.needsUpdate = true;
  s.sea.geometry.computeVertexNormals();
}

export default function TerrainView({ preview, extentM, onInfo, sculpt, seaLevelM = null, water = null, waves = null, sky = null, horizon = null, sun, overlay = null, instances = null, maps = null }: Props) {
  const host = useRef<HTMLDivElement>(null);
  const sceneRef = useRef<SceneState | null>(null);
  // Latest sculpt props without re-creating the renderer effect.
  const sculptRef = useRef(sculpt);
  sculptRef.current = sculpt;
  const extentRef = useRef(extentM);
  extentRef.current = extentM;

  useEffect(() => {
    const el = host.current;
    if (!el) return;
    const renderer = new WebGPURenderer({ antialias: true });
    renderer.setPixelRatio(window.devicePixelRatio);
    renderer.setSize(el.clientWidth, el.clientHeight);
    el.appendChild(renderer.domElement);
    const scene = new THREE.Scene();
    // Sky: a gradient dome (zenith blue → pale horizon, vertex colours, no texture asset) plus
    // distance fog in the horizon colour so scale reads. The dome ignores fog and depth.
    scene.background = new THREE.Color(0x1b2432);
    scene.fog = new THREE.Fog(0xb9c7d9, 1, 2);
    const camera = new THREE.PerspectiveCamera(50, el.clientWidth / Math.max(1, el.clientHeight), 1, 200000);
    const hemi = new THREE.HemisphereLight(0xdfe8ff, 0x3a3630, 0.9);
    scene.add(hemi);
    const sun = new THREE.DirectionalLight(0xffffff, 1.6);
    sun.position.set(3000, 5000, 2000);
    scene.add(sun);
    const skyGeo = new THREE.SphereGeometry(1, 32, 16);
    {
      const pos = skyGeo.attributes.position as THREE.BufferAttribute;
      const colors = new Float32Array(pos.count * 3);
      const zenith = new THREE.Color(0x24467a);
      const horizon = new THREE.Color(0xb9c7d9);
      // Below the horizon the dome goes dark quickly (the old dusk background) so the terrain
      // keeps its contrast when the camera looks down at it.
      const ground = new THREE.Color(0x1b2432);
      const c = new THREE.Color();
      for (let i = 0; i < pos.count; i++) {
        const y = pos.getY(i); // -1..1
        if (y >= 0) c.copy(horizon).lerp(zenith, Math.pow(y, 0.6));
        else c.copy(horizon).lerp(ground, Math.min(1, -y * 10));
        colors[i * 3] = c.r;
        colors[i * 3 + 1] = c.g;
        colors[i * 3 + 2] = c.b;
      }
      skyGeo.setAttribute("color", new THREE.BufferAttribute(colors, 3));
    }
    const sky = new THREE.Mesh(skyGeo, new THREE.MeshBasicMaterial({ vertexColors: true, side: THREE.BackSide, fog: false, depthWrite: false }));
    sky.scale.setScalar(50000);
    sky.renderOrder = -1;
    scene.add(sky);
    // Brush cursor: a flat ring (unit radius, scaled per brush) lifted slightly above the surface.
    const brush = new THREE.Mesh(
      new THREE.RingGeometry(0.9, 1.0, 48),
      new THREE.MeshBasicMaterial({ color: 0xff3b30, transparent: true, opacity: 0.85, side: THREE.DoubleSide, depthTest: false }),
    );
    brush.rotation.x = -Math.PI / 2;
    brush.visible = false;
    brush.renderOrder = 10;
    scene.add(brush);
    const state: SceneState = {
      renderer,
      scene,
      camera,
      mesh: null,
      brush,
      frame: 0,
      backend: "none",
      triangles: 0,
      sea: null,
      seaWaves: null,
      sun,
      hemi,
      sky,
      sunDeg: { azimuthDeg: 140, elevationDeg: 35 },
      orbit: { theta: 0.6, phi: 1.0, radius: 5200, minR: 200, maxR: 100000 },
      instances: null,
      pbrPx: null,
      skyPx: null,
      skirt: null,
    };
    sceneRef.current = state;
    placeCamera(state);

    // Simple orbit: drag to rotate, wheel to zoom (no OrbitControls dependency so it works on both backends).
    // In sculpt mode the drag is a brush stroke: the pointer is raycast onto the terrain mesh.
    let dragging = false;
    let stroke: [number, number][] | null = null;
    const raycaster = new THREE.Raycaster();
    const hit = (e: PointerEvent): [number, number] | null => {
      const ext = extentRef.current;
      if (!state.mesh || !ext) return null;
      const r = el.getBoundingClientRect();
      raycaster.setFromCamera(new THREE.Vector2(((e.clientX - r.left) / r.width) * 2 - 1, -((e.clientY - r.top) / r.height) * 2 + 1), camera);
      const h = raycaster.intersectObject(state.mesh, false)[0];
      if (!h) return null;
      // Mesh is centred at the origin; layer row 0 is SOUTH (+Z in the viewport), so v counts
      // from the south edge: v = 0 at +Z/2.
      return [h.point.x + ext[0] / 2, ext[1] / 2 - h.point.z];
    };
    // Brush cursor follows the pointer over the terrain while sculpting (also when not dragging).
    const showBrush = (e: PointerEvent) => {
      const sc = sculptRef.current;
      const ext = extentRef.current;
      if (!sc?.enabled || !state.mesh || !ext) {
        state.brush.visible = false;
        el.removeAttribute("data-brush");
        return;
      }
      const p = hit(e);
      if (!p) {
        state.brush.visible = false;
        el.removeAttribute("data-brush");
        return;
      }
      const x = p[0] - ext[0] / 2;
      const z = ext[1] / 2 - p[1];
      // Sit just above the surface so the ring is not z-fought by the mesh (depthTest is off anyway).
      const y = raycaster.intersectObject(state.mesh, false)[0]?.point.y ?? 0;
      state.brush.position.set(x, y + ext[0] * 0.002, z);
      state.brush.scale.setScalar(Math.max(1e-3, sc.radiusM));
      state.brush.visible = true;
      el.setAttribute("data-brush", `${p[0].toFixed(1)},${p[1].toFixed(1)}`);
    };
    const onDown = (e: PointerEvent) => {
      dragging = true;
      if (sculptRef.current?.enabled) {
        const p = hit(e);
        stroke = p ? [p] : [];
      }
    };
    const onUp = () => {
      dragging = false;
      if (stroke) {
        if (stroke.length > 0) sculptRef.current?.onStroke(stroke);
        stroke = null;
      }
    };
    const onMove = (e: PointerEvent) => {
      showBrush(e);
      if (!dragging) return;
      if (stroke) {
        const p = hit(e);
        const last = stroke[stroke.length - 1];
        // Coalesce samples closer than a few metres so a stroke stays a few dozen points.
        if (p && (!last || Math.hypot(p[0] - last[0], p[1] - last[1]) > (extentRef.current?.[0] ?? 1000) * 0.005)) stroke.push(p);
        return;
      }
      state.orbit.theta -= e.movementX * 0.005;
      state.orbit.phi = Math.min(1.5, Math.max(0.15, state.orbit.phi - e.movementY * 0.005));
      placeCamera(state);
    };
    const onWheel = (e: WheelEvent) => {
      const o = state.orbit;
      o.radius = Math.min(o.maxR, Math.max(o.minR, o.radius * (1 + e.deltaY * 0.001)));
      placeCamera(state);
      e.preventDefault();
    };
    el.addEventListener("pointerdown", onDown);
    window.addEventListener("pointerup", onUp);
    el.addEventListener("pointermove", onMove);
    el.addEventListener("wheel", onWheel, { passive: false });
    const onResize = () => {
      renderer.setSize(el.clientWidth, el.clientHeight);
      camera.aspect = el.clientWidth / Math.max(1, el.clientHeight);
      camera.updateProjectionMatrix();
    };
    const ro = new ResizeObserver(onResize);
    ro.observe(el);

    let alive = true;
    renderer.init().then(() => {
      if (!alive) return;
      // The backend is only known after init (WebGPU, or the WebGL 2 fallback).
      state.backend = isWebGPU(renderer) ? "webgpu" : "webgl2";
      onInfo?.(infoOf(state));
      const loop = () => {
        if (!alive) return;
        animateSea(state, performance.now() / 1000);
        renderer.render(scene, camera);
        state.frame = requestAnimationFrame(loop);
      };
      loop();
    });
    return () => {
      alive = false;
      cancelAnimationFrame(state.frame);
      ro.disconnect();
      el.removeEventListener("pointerdown", onDown);
      window.removeEventListener("pointerup", onUp);
      el.removeEventListener("pointermove", onMove);
      el.removeEventListener("wheel", onWheel);
      state.brush.geometry.dispose();
      state.brush.material.dispose();
      if (state.mesh) {
        state.mesh.geometry.dispose();
        state.mesh.material.dispose();
      }
      if (state.instances) {
        state.instances.geometry.dispose();
        state.instances.material.dispose();
      }
      renderer.dispose();
      el.removeChild(renderer.domElement);
      sceneRef.current = null;
    };
  }, [onInfo]);

  useEffect(() => {
    const s = sceneRef.current;
    if (!s) return;
    if (s.mesh) {
      s.scene.remove(s.mesh);
      s.mesh.geometry.dispose();
      for (const t of [s.mesh.material.map, s.mesh.material.normalMap, s.mesh.material.roughnessMap]) t?.dispose();
      s.mesh.material.dispose();
      s.mesh = null;
      s.triangles = 0;
    }
    if (!preview || !extentM) return;
    const geo = buildTerrainGeometry(preview, extentM, overlay);
    // Baked maps texture the terrain like the exports (full-field relief at any preview LOD);
    // a colour overlay wins, and without maps the hypsometric vertex colours show.
    const textured = maps && !overlay;
    const tex = (img: ImageBitmap, color: boolean) => {
      const t = new THREE.Texture(img);
      if (color) t.colorSpace = THREE.SRGBColorSpace;
      t.needsUpdate = true;
      return t;
    };
    const mat = textured
      ? new THREE.MeshStandardMaterial({ map: tex(maps.albedo, true), normalMap: tex(maps.normal, false), roughnessMap: tex(maps.roughness, false), roughness: 1, metalness: 0 })
      : new THREE.MeshStandardMaterial({ vertexColors: true, roughness: 0.95, metalness: 0 });
    s.pbrPx = textured ? maps.px : null;
    s.mesh = new THREE.Mesh(geo, mat);
    s.scene.add(s.mesh);
    s.triangles = (preview.width - 1) * (preview.height - 1) * 2;
    // Camera distance and clip planes follow the world's extent so any size frames correctly.
    const size = Math.max(extentM[0], extentM[1]);
    s.orbit.radius = size * 1.3;
    s.orbit.minR = size * 0.02;
    s.orbit.maxR = size * 20;
    s.camera.near = Math.max(0.1, size * 0.001);
    s.camera.far = size * 50;
    s.sky.scale.setScalar(size * 40);
    if (s.scene.fog instanceof THREE.Fog) {
      s.scene.fog.near = size * 2;
      s.scene.fog.far = size * 12;
    }
    s.camera.updateProjectionMatrix();
    placeCamera(s);
    onInfo?.(infoOf(s));
  }, [preview, extentM, overlay, maps, onInfo]);

  // Sun: direction from azimuth/elevation; low sun goes warm and dimmer, the sky light follows.
  useEffect(() => {
    const s = sceneRef.current;
    if (!s) return;
    const az = ((sun?.azimuthDeg ?? 140) * Math.PI) / 180;
    const el = ((sun?.elevationDeg ?? 35) * Math.PI) / 180;
    const r = 10000;
    // Azimuth is where the light comes FROM, clockwise from north (-Z).
    s.sun.position.set(r * Math.cos(el) * Math.sin(az), r * Math.sin(el), -r * Math.cos(el) * Math.cos(az));
    const warm = Math.min(1, Math.max(0, (20 - (sun?.elevationDeg ?? 35)) / 20));
    s.sun.color.set(0xffffff).lerp(new THREE.Color(0xffa054), warm);
    s.sun.intensity = 1.6 * (0.55 + 0.45 * Math.sin(el));
    s.hemi.intensity = 0.9 * (0.6 + 0.4 * Math.sin(el));
    s.sunDeg = { azimuthDeg: sun?.azimuthDeg ?? 140, elevationDeg: sun?.elevationDeg ?? 35 };
    onInfo?.(infoOf(s));
  }, [sun?.azimuthDeg, sun?.elevationDeg, onInfo]);

  // Horizon skirt: the noise continued beyond the border, at `scale` × the extent, with the
  // triangles under the world itself dropped so the real terrain shows through the hole.
  useEffect(() => {
    const s = sceneRef.current;
    if (!s) return;
    if (s.skirt) {
      s.scene.remove(s.skirt);
      s.skirt.geometry.dispose();
      s.skirt.material.dispose();
      s.skirt = null;
    }
    if (horizon && extentM) {
      const far: [number, number] = [extentM[0] * horizon.scale, extentM[1] * horizon.scale];
      const geo = buildTerrainGeometry(horizon.preview, far);
      const pos = geo.attributes.position as THREE.BufferAttribute;
      const idx = geo.index!;
      const kept: number[] = [];
      for (let t = 0; t < idx.count; t += 3) {
        let cx = 0;
        let cz = 0;
        for (let k = 0; k < 3; k++) {
          cx += pos.getX(idx.getX(t + k));
          cz += pos.getZ(idx.getX(t + k));
        }
        if (Math.abs(cx / 3) > extentM[0] / 2 || Math.abs(cz / 3) > extentM[1] / 2) kept.push(idx.getX(t), idx.getX(t + 1), idx.getX(t + 2));
      }
      geo.setIndex(kept);
      const mat = new THREE.MeshStandardMaterial({ vertexColors: true, roughness: 0.95, metalness: 0.0 });
      mat.color.setScalar(0.85); // a touch dimmer: it is the far distance
      s.skirt = new THREE.Mesh(geo, mat);
      s.skirt.renderOrder = -0.5;
      s.scene.add(s.skirt);
    }
    onInfo?.(infoOf(s));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [horizon, extentM, onInfo]);

  // Sky dome: the physical sky preview as an equirectangular texture. three's sphere UVs run
  // west → south → east → north around +Y and v = 1 at the zenith; the preview's columns run
  // north → east → south → west and its first row is the zenith, so the lookup is mirrored and
  // offset through the texture's repeat/offset instead of rewriting the geometry's UVs.
  useEffect(() => {
    const s = sceneRef.current;
    if (!s) return;
    const mat = s.sky.material;
    mat.map?.dispose();
    if (sky) {
      const tex = new THREE.CanvasTexture(sky.bitmap);
      tex.colorSpace = THREE.SRGBColorSpace;
      tex.wrapS = THREE.RepeatWrapping;
      tex.wrapT = THREE.ClampToEdgeWrapping;
      tex.repeat.set(-1, 1); // mirror the azimuth; v = 1 is the image's top row (zenith) after the default flip
      tex.offset.set(0.75, 0);
      tex.needsUpdate = true;
      mat.map = tex;
      mat.vertexColors = false;
      s.skyPx = sky.px;
    } else {
      mat.map = null;
      mat.vertexColors = true;
      s.skyPx = null;
    }
    mat.needsUpdate = true;
    onInfo?.(infoOf(s));
  }, [sky, onInfo]);

  // Sea: follows the graph's sea-level node; a flat surface whose colour and opacity follow the
  // depth of the terrain under it (shoreline foam, translucent shallows, opaque deep water).
  useEffect(() => {
    const s = sceneRef.current;
    if (!s) return;
    if (s.sea) {
      s.scene.remove(s.sea);
      s.sea.geometry.dispose();
      s.sea.material.dispose();
      s.sea = null;
    }
    if (seaLevelM !== null && extentM && preview) {
      const geo = buildSeaGeometry(preview, extentM, seaLevelM, water ?? DEFAULT_WATER);
      const mat = new THREE.MeshStandardMaterial({ vertexColors: true, transparent: true, roughness: 0.2, metalness: 0.05, depthWrite: false });
      s.sea = new THREE.Mesh(geo, mat);
      s.sea.position.y = seaLevelM;
      s.scene.add(s.sea);
      s.seaWaves = waves ? { base: Float32Array.from(geo.attributes.position!.array as Float32Array), components: waves.components, heightM: waves.significant_height_m, lengthM: waves.wavelength_m } : null;
    } else {
      s.seaWaves = null;
    }
    onInfo?.(infoOf(s));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [seaLevelM, water?.visibilityM, water?.tint?.[0], water?.tint?.[1], water?.tint?.[2], water?.breakingDepthM, waves, extentM, preview, onInfo]);

  // Scatter instances: one InstancedMesh of cones in the terrain frame (x = u − extent/2, z = extent/2 − v),
  // yaw and a 0.75..1.25 scale from `r` exactly as the GLB export writes them.
  useEffect(() => {
    const s = sceneRef.current;
    if (!s) return;
    if (s.instances) {
      s.scene.remove(s.instances);
      s.instances.geometry.dispose();
      s.instances.material.dispose();
      s.instances = null;
    }
    if (instances && instances.points.length > 0 && extentM) {
      const geo = new THREE.ConeGeometry(0.3, 1, 6);
      geo.translate(0, 0.5, 0); // base on the ground
      const mesh = new THREE.InstancedMesh(geo, new THREE.MeshStandardMaterial({ color: 0x3c9a4c, roughness: 0.8 }), instances.points.length);
      const m = new THREE.Matrix4();
      const q = new THREE.Quaternion();
      const pos = new THREE.Vector3();
      const scl = new THREE.Vector3();
      const up = new THREE.Vector3(0, 1, 0);
      instances.points.forEach((p, i) => {
        const k = instances.sizeM * (0.75 + 0.5 * ((p.r * 7.31) % 1));
        m.compose(pos.set(p.u - extentM[0] / 2, p.z, extentM[1] / 2 - p.v), q.setFromAxisAngle(up, 2 * Math.PI * p.r), scl.set(k, k, k));
        mesh.setMatrixAt(i, m);
      });
      mesh.instanceMatrix.needsUpdate = true;
      mesh.frustumCulled = false; // the base geometry's bounds do not cover the instances
      s.instances = mesh;
      s.scene.add(mesh);
    }
    onInfo?.(infoOf(s));
    // Depend on the parts, not the wrapper object: App builds a fresh `{ points, sizeM }` every
    // render and onInfo → setState would otherwise loop ("Maximum update depth exceeded").
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [instances?.points, instances?.sizeM, extentM, onInfo]);

  return <div ref={host} className="h-full w-full" data-testid="viewport" />;
}
