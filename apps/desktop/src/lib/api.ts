// Thin typed client over the Tauri commands. Mirrors docs/spec/json-api.md.
import { invoke } from "@tauri-apps/api/core";

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

/** Invoke a registry operation. Never throws for domain errors: inspect `status`. */
export async function op<T = unknown>(id: string, params: Record<string, unknown> = {}): Promise<Response<T>> {
  return invoke<Response<T>>("op", { request: { op: id, schema_version: SCHEMA_VERSION, params } });
}

/** A node's output as a small preview (`graph.node_preview`): the same wire format as a layer preview. */
export async function nodePreview(id: string, port = "out"): Promise<Preview | null> {
  const r = await op<{ bytes_base64: string }>("graph.node_preview", { id, port });
  if (r.status !== "ok" || !r.result) return null;
  const bin = atob(r.result.bytes_base64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return decodePreview(bytes.buffer);
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
