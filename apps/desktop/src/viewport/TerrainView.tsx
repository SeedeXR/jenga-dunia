// Terrain viewport: three.js WebGPURenderer with automatic WebGL 2 fallback (ADR-0003).
// The mesh is a regular grid displaced on the CPU from the u16 preview; colours follow height.
import { useEffect, useRef } from "react";
import * as THREE from "three";
import { WebGPURenderer } from "three/webgpu";
import type { Preview } from "../lib/api";

export interface ViewportInfo {
  backend: "webgpu" | "webgl2" | "none";
  triangles: number;
}

interface Props {
  preview: Preview | null;
  extentM: [number, number] | null;
  onInfo?: (info: ViewportInfo) => void;
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
    colors[vi * 3] += (ROCK.r - (colors[vi * 3] ?? 0)) * slope;
    colors[vi * 3 + 1] += (ROCK.g - (colors[vi * 3 + 1] ?? 0)) * slope;
    colors[vi * 3 + 2] += (ROCK.b - (colors[vi * 3 + 2] ?? 0)) * slope;
  }
  geo.setAttribute("color", new THREE.BufferAttribute(colors, 3));
  return geo;
}

interface SceneState {
  renderer: WebGPURenderer;
  scene: THREE.Scene;
  camera: THREE.PerspectiveCamera;
  mesh: THREE.Mesh<THREE.BufferGeometry, THREE.MeshStandardMaterial> | null;
  frame: number;
  backend: ViewportInfo["backend"];
  triangles: number;
  orbit: { theta: number; phi: number; radius: number; minR: number; maxR: number };
}

function placeCamera(s: SceneState) {
  const { theta, phi, radius } = s.orbit;
  s.camera.position.set(radius * Math.sin(phi) * Math.sin(theta), radius * Math.cos(phi), radius * Math.sin(phi) * Math.cos(theta));
  s.camera.lookAt(0, 0, 0);
}

export default function TerrainView({ preview, extentM, onInfo }: Props) {
  const host = useRef<HTMLDivElement>(null);
  const sceneRef = useRef<SceneState | null>(null);

  useEffect(() => {
    const el = host.current;
    if (!el) return;
    const renderer = new WebGPURenderer({ antialias: true });
    renderer.setPixelRatio(window.devicePixelRatio);
    renderer.setSize(el.clientWidth, el.clientHeight);
    el.appendChild(renderer.domElement);
    const scene = new THREE.Scene();
    // Sky: a dusk-blue background with distance fog so scale reads without a skybox asset.
    scene.background = new THREE.Color(0x1b2432);
    scene.fog = new THREE.Fog(0x1b2432, 1, 2);
    const camera = new THREE.PerspectiveCamera(50, el.clientWidth / Math.max(1, el.clientHeight), 1, 200000);
    scene.add(new THREE.HemisphereLight(0xdfe8ff, 0x202020, 0.9));
    const sun = new THREE.DirectionalLight(0xffffff, 1.6);
    sun.position.set(3000, 5000, 2000);
    scene.add(sun);
    const state: SceneState = {
      renderer,
      scene,
      camera,
      mesh: null,
      frame: 0,
      backend: "none",
      triangles: 0,
      orbit: { theta: 0.6, phi: 1.0, radius: 5200, minR: 200, maxR: 100000 },
    };
    sceneRef.current = state;
    placeCamera(state);

    // Simple orbit: drag to rotate, wheel to zoom (no OrbitControls dependency so it works on both backends).
    let dragging = false;
    const onDown = () => (dragging = true);
    const onUp = () => (dragging = false);
    const onMove = (e: PointerEvent) => {
      if (!dragging) return;
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
      onInfo?.({ backend: state.backend, triangles: state.triangles });
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
    if (s.scene.fog instanceof THREE.Fog) {
      s.scene.fog.near = size * 2;
      s.scene.fog.far = size * 12;
    }
    s.camera.updateProjectionMatrix();
    placeCamera(s);
    onInfo?.({ backend: s.backend, triangles: s.triangles });
  }, [preview, extentM, onInfo]);

  return <div ref={host} className="h-full w-full" data-testid="viewport" />;
}
