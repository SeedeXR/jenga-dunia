// Thin typed client over the Tauri commands. Mirrors docs/spec/json-api.md.
import { invoke } from "@tauri-apps/api/core";
import type { JdOps, MaterialSpec } from "./ops.gen";

export const SCHEMA_VERSION = 1;

export interface JdError {
  code: string;
  message: string;
  why?: string;
  next?: string;
  details?: Record<string, unknown>;
}

export interface Warning {
  code: string;
  message: string;
  details?: unknown;
}

export interface Response<T = unknown> {
  op: string;
  schema_version: number;
  request_id: string;
  status: "ok" | "accepted" | "error";
  result?: T;
  job_id?: number;
  error?: JdError;
  warnings?: Warning[];
}

export interface OperationInfo {
  id: string;
  description: string;
  scope: string;
  request_schema: unknown;
  result_schema: unknown;
}

/** Invoke a registry operation. Never throws for domain errors: inspect `status`. */
export async function op<T = unknown>(id: string, params: Record<string, unknown> = {}): Promise<Response<T>> {
  return invoke<Response<T>>("op", { request: { op: id, schema_version: SCHEMA_VERSION, params } });
}

export async function opsList(): Promise<OperationInfo[]> {
  return invoke<OperationInfo[]>("ops_list");
}

export interface Preview {
  width: number;
  height: number;
  min: number;
  max: number;
  data: Uint16Array;
}

export const PREVIEW_HEADER_BYTES = 16;

/** Decode the wire format written by jd-terrain `Preview::to_bytes` (little-endian). */
export function decodePreview(buf: ArrayBuffer): Preview {
  if (buf.byteLength < PREVIEW_HEADER_BYTES) throw new Error("preview buffer too short");
  const view = new DataView(buf);
  const width = view.getUint32(0, true);
  const height = view.getUint32(4, true);
  const min = view.getFloat32(8, true);
  const max = view.getFloat32(12, true);
  const n = width * height;
  if (buf.byteLength < PREVIEW_HEADER_BYTES + n * 2) throw new Error("preview buffer truncated");
  // Copy so the typed array is 2-byte aligned regardless of the source buffer.
  const data = new Uint16Array(n);
  const body = new DataView(buf, PREVIEW_HEADER_BYTES, n * 2);
  for (let i = 0; i < n; i++) data[i] = body.getUint16(i * 2, true);
  return { width, height, min, max, data };
}

/** Fetch a layer preview as raw bytes (no JSON encoding of the buffer). */
export async function layerPreview(layer: string, lod: number): Promise<{ preview: Preview; bytes: number; ms: number }> {
  const t0 = performance.now();
  const buf = await invoke<ArrayBuffer>("layer_preview", { layer, lod });
  const ms = performance.now() - t0;
  return { preview: decodePreview(buf), bytes: buf.byteLength, ms };
}

/** Material maps of a layer as decoded bitmaps (albedo, tangent-space normal, roughness). */
export interface DetailTile {
  layer: number;
  tileM: number;
  px: number;
  albedo: ImageBitmap;
  normal: ImageBitmap;
}

export interface LayerMaps {
  albedo: ImageBitmap;
  normal: ImageBitmap;
  roughness: ImageBitmap;
  px: [number, number];
  /** Detail tiles of the layers that have one, and their masks packed RGBA at the map size. */
  details: DetailTile[];
  detailWeights: ImageBitmap | null;
}

/** The noise continued `scale` × beyond the world's border as a coarse preview (the horizon skirt). */
export async function horizonPreview(scale = 3, px = 256): Promise<Preview | null> {
  const r = await op<{ bytes_base64: string }>("terrain.horizon", { scale, px });
  if (r.status !== "ok" || !r.result) return null;
  const bin = Uint8Array.from(atob(r.result.bytes_base64), (c) => c.charCodeAt(0));
  return decodePreview(bin.buffer);
}

/** The physical sky for the viewport dome: a tonemapped equirectangular PNG decoded to a bitmap. */
export async function skyPreview(sun: { sun_azimuth_deg: number; sun_elevation_deg: number; turbidity: number; cloud_cover?: number }, width = 512): Promise<{ bitmap: ImageBitmap; px: [number, number] } | null> {
  // Three times the bake's default exposure: the dome is seen tonemapped, and a clear zenith near 0.5 reads grey.
  const r = await op<{ width: number; height: number; png_base64: string }>("sky.preview", { ...sun, exposure: 0.15, width });
  if (r.status !== "ok" || !r.result) return null;
  const bitmap = await createImageBitmap(new Blob([Uint8Array.from(atob(r.result.png_base64), (c) => c.charCodeAt(0))], { type: "image/png" }));
  return { bitmap, px: [r.result.width, r.result.height] };
}

export async function layerTextures(layer: string, maxPx = 1024, materials: MaterialSpec[] = []): Promise<{ maps: LayerMaps; ms: number } | null> {
  const t0 = performance.now();
  const r = await op<JdOps["terrain.textures"]["result"]>("terrain.textures", { materials, layer, max_px: maxPx });
  if (r.status !== "ok" || !r.result) return null;
  const decode = (b64: string) => createImageBitmap(new Blob([Uint8Array.from(atob(b64), (c) => c.charCodeAt(0))], { type: "image/png" }));
  const [albedo, normal, roughness] = await Promise.all([decode(r.result.albedo_png_base64), decode(r.result.normal_png_base64), decode(r.result.roughness_png_base64)]);
  const details: DetailTile[] = await Promise.all(
    (r.result.details ?? []).map(async (d) => ({ layer: d.layer, tileM: d.tile_m, px: d.px, albedo: await decode(d.albedo_png_base64), normal: await decode(d.normal_png_base64) })),
  );
  const detailWeights = r.result.detail_weights_png_base64 ? await decode(r.result.detail_weights_png_base64) : null;
  return { maps: { albedo, normal, roughness, px: [r.result.width, r.result.height], details, detailWeights }, ms: performance.now() - t0 };
}
