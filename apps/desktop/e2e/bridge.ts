// Bridges the frontend's Tauri `invoke` to the real Rust core: every `op` call becomes an MCP
// `tools/call` on a spawned `jd-mcp`, so the UI is exercised against the same code the desktop app
// and agents use. Only the transport differs from the Tauri shell.
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";
import type { Page } from "@playwright/test";
import { existsSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));

export function mcpBinary(): string {
  const fromEnv = process.env.JD_MCP_BIN;
  if (fromEnv) return fromEnv;
  const root = path.resolve(here, "../../..");
  const exe = process.platform === "win32" ? ".exe" : "";
  // Prefer the most recently built binary so a stale release build never shadows a fresh debug one.
  const candidates = ["target/release/jd-mcp", "target/debug/jd-mcp"]
    .map((p) => path.join(root, p + exe))
    .filter((p) => existsSync(p))
    .sort((a, b) => statSync(b).mtimeMs - statSync(a).mtimeMs);
  if (candidates[0]) return candidates[0];
  throw new Error("jd-mcp binary not found; run `cargo build -p jd-mcp` or set JD_MCP_BIN");
}

export async function connectCore(workspace: string): Promise<Client> {
  const transport = new StdioClientTransport({
    command: mcpBinary(),
    args: ["--scope", "all", "--workspace", workspace],
    env: { ...process.env, JD_LOG: "warn" } as Record<string, string>,
  });
  const client = new Client({ name: "jd-e2e", version: readFileSync(new URL("../../../VERSION", import.meta.url), "utf8").trim() });
  await client.connect(transport);
  return client;
}

type Envelope = { status: string; result?: Record<string, unknown>; error?: unknown };

export async function installBridge(page: Page, core: Client): Promise<void> {
  await page.exposeFunction("__jd_invoke", async (cmd: string, args: Record<string, unknown>) => {
    if (cmd.startsWith("plugin:event|")) return { kind: "json", value: 0 }; // no native menu in the browser
    if (cmd === "ops_list") {
      const { tools } = await core.listTools();
      return { kind: "json", value: tools.map((t) => ({ id: t.name, description: t.description, scope: "", request_schema: t.inputSchema, result_schema: t.outputSchema })) };
    }
    if (cmd === "layer_preview") {
      const r = await core.callTool({ name: "terrain.preview", arguments: { layer: args.layer, lod: args.lod } });
      const env = r.structuredContent as Envelope;
      if (env.status !== "ok" || !env.result) return { kind: "error", value: env };
      // `bytes_base64` is exactly the wire format the Tauri command returns (header + u16 body).
      return { kind: "preview", b64: (env.result as { bytes_base64: string }).bytes_base64 };
    }
    if (cmd === "op") {
      const req = (args as { request: { op: string; params: Record<string, unknown> } }).request;
      const r = await core.callTool({ name: req.op, arguments: req.params });
      return { kind: "json", value: r.structuredContent };
    }
    throw new Error(`unbridged command ${cmd}`);
  });
  await page.addInitScript(() => {
    const b64ToBuf = (b64: string) => {
      const bin = atob(b64);
      const out = new Uint8Array(bin.length);
      for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
      return out.buffer;
    };
    const w = window as unknown as {
      __jd_invoke: (c: string, a: unknown) => Promise<Record<string, unknown>>;
      __TAURI_INTERNALS__: unknown;
      __TAURI_EVENT_PLUGIN_INTERNALS__: unknown;
    };
    // The event module (menu listener) unregisters through this global when `unlisten` runs.
    w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => undefined };
    w.__TAURI_INTERNALS__ = {
      invoke: async (cmd: string, args: unknown) => {
        const r = await w.__jd_invoke(cmd, args);
        if (r.kind === "json") return r.value;
        if (r.kind === "error") throw r.value;
        return b64ToBuf(r.b64 as string);
      },
      transformCallback: () => 0,
      unregisterCallback: () => undefined,
      convertFileSrc: (p: string) => p,
    };
  });
}
