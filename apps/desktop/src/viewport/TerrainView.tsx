// Terrain viewport: three.js WebGPURenderer with automatic WebGL 2 fallback (ADR-0003).
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
}

interface Props {
  preview: Preview | null;
  extentM: [number, number] | null;
  onInfo?: (info: ViewportInfo) => void;
  /** Sculpt mode: pointer drags paint a stroke on the terrain instead of orbiting. Points are
   *  reported in layer metres (u east from the west edge, v north from the south edge = row 0). */
  sculpt?: { enabled: boolean; radiusM: number; onStroke: (points: [number, number][]) => void };
  /** Draw a translucent water plane at this height (metres); null hides it. */
  seaLevelM?: number | null;
  /** Sun direction: azimuth clockwise from north (0 = light from the north), elevation above the horizon. */
  sun?: { azimuthDeg: number; elevationDeg: number };
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

export function buildTerrainGeometry(p: Preview, extent: [number, number]): THREE.BufferGeometry {
  const { width, height, min, max, data } = p;
  const geo = new THREE.PlaneGeometry(extent[0], extent[1], width - 1, height - 1);
  geo.rotateX(-Math.PI / 2); // plane in XZ, +Y up (glTF frame)
  const pos = geo.attributes.position as THREE.BufferAttribute;
  const colors = new Float32Array(pos.count * 3);
  const range = max - min || 1;
  for (let j = 0; j < height; j++) {
    // PlaneGeometry rows run from +Z (front) to -Z; the preview's row 0 is south (+Z after rotation).
    for (let i = 0; i < width; i++) {
      const vi = j * width + i;
      const q = data[(height - 1 - j) * width + i] ?? 0;
      pos.setY(vi, min + (q / 65535) * range);
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
  for (let vi = 0; vi < pos.count; vi++) {
    const slope = Math.min(1, Math.max(0, (1 - nrm.getY(vi)) * 1.4));
    const [r, g, b] = [colors[vi * 3] ?? 0, colors[vi * 3 + 1] ?? 0, colors[vi * 3 + 2] ?? 0];
    colors[vi * 3] = r + (ROCK.r - r) * slope;
    colors[vi * 3 + 1] = g + (ROCK.g - g) * slope;
    colors[vi * 3 + 2] = b + (ROCK.b - b) * slope;
  }
  geo.setAttribute("color", new THREE.BufferAttribute(colors, 3));
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
  sea: THREE.Mesh<THREE.PlaneGeometry, THREE.MeshStandardMaterial> | null;
  sun: THREE.DirectionalLight;
  hemi: THREE.HemisphereLight;
  /** Gradient sky dome (vertex colours, inside faces). */
  sky: THREE.Mesh<THREE.SphereGeometry, THREE.MeshBasicMaterial>;
  sunDeg: { azimuthDeg: number; elevationDeg: number };
  orbit: { theta: number; phi: number; radius: number; minR: number; maxR: number };
}

function placeCamera(s: SceneState) {
  const { theta, phi, radius } = s.orbit;
  s.camera.position.set(radius * Math.sin(phi) * Math.sin(theta), radius * Math.cos(phi), radius * Math.sin(phi) * Math.cos(theta));
  s.camera.lookAt(0, 0, 0);
}

/** One report shape for every effect, so a later report never drops a field an earlier one set. */
function infoOf(s: SceneState): ViewportInfo {
  return { backend: s.backend, triangles: s.triangles, ...(s.sea ? { seaM: s.sea.position.y } : {}), sun: s.sunDeg };
}

export default function TerrainView({ preview, extentM, onInfo, sculpt, seaLevelM = null, sun }: Props) {
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
      sun,
      hemi,
      sky,
      sunDeg: { azimuthDeg: 140, elevationDeg: 35 },
      orbit: { theta: 0.6, phi: 1.0, radius: 5200, minR: 200, maxR: 100000 },
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
      s.mesh.material.dispose();
      s.mesh = null;
      s.triangles = 0;
    }
    if (!preview || !extentM) return;
    const geo = buildTerrainGeometry(preview, extentM);
    const mat = new THREE.MeshStandardMaterial({ vertexColors: true, roughness: 0.95, metalness: 0 });
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
  }, [preview, extentM, onInfo]);

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

  // Sea plane: follows the graph's sea-level node; a flat translucent quad over the whole world.
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
      const geo = new THREE.PlaneGeometry(extentM[0], extentM[1]);
      geo.rotateX(-Math.PI / 2);
      const mat = new THREE.MeshStandardMaterial({ color: 0x2f6fd0, transparent: true, opacity: 0.6, roughness: 0.25, metalness: 0.1 });
      s.sea = new THREE.Mesh(geo, mat);
      s.sea.position.y = seaLevelM;
      s.scene.add(s.sea);
    }
    onInfo?.(infoOf(s));
  }, [seaLevelM, extentM, preview, onInfo]);

  return <div ref={host} className="h-full w-full" data-testid="viewport" />;
}
