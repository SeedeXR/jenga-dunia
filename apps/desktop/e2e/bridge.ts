// Bridges the frontend's Tauri `invoke` to the real Rust core: every `op` call becomes an MCP
// `tools/call` on a spawned `jd-mcp`, so the UI is exercised against the same code the desktop app
// and agents use. Only the transport differs from the Tauri shell.
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";
import { expect, type Locator, type Page } from "@playwright/test";
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

/** Drag a connection in the React Flow graph from `from`'s source handle to `to`'s target handle. */
export async function wire(page: Page, from: Locator, to: Locator, fromHandle = "out", toHandle?: string): Promise<void> {
  const src = from.locator(`.react-flow__handle.source[data-handleid="${fromHandle}"]`);
  // Nodes with several inputs need the target handle named (strict mode otherwise sees them all).
  const dst = toHandle ? to.locator(`.react-flow__handle.target[data-handleid="${toHandle}"]`) : to.locator(".react-flow__handle.target");
  // React Flow cannot scroll a node into view: an off-canvas handle makes the forced click land on
  // whatever is under its coordinates (seen as a silently missing edge). Fail here instead.
  await expect(src).toBeInViewport();
  await expect(dst).toBeInViewport();
  // React Flow's connect-on-click: click the source handle, then the target handle. A drag works
  // at full zoom but lands short on the few-pixel handles after a fit-view; the click path is
  // zoom-independent, which is what a scripted user needs.
  const before = await logCount(page, "graph.connect: ok");
  await src.click({ force: true });
  await dst.click({ force: true });
  await expect.poll(() => logCount(page, "graph.connect: ok")).toBeGreaterThan(before);
}

/** Click React Flow's fit-view until `node` is on screen: a node added a moment ago has no measured
 *  size yet, and a fit that runs before the measurement leaves it off-canvas. */
export async function fitView(page: Page, node: Locator): Promise<void> {
  const button = page.getByTestId("graph-panel").locator(".react-flow__controls-fitview");
  for (let i = 0; ; i++) {
    await button.click();
    try {
      await expect(node).toBeInViewport({ timeout: 500 });
      return;
    } catch (e) {
      if (i >= 9) throw e;
    }
  }
}

/** How many lines of the op log contain `text` (assert on a count, not on text that an earlier op already logged). */
export function logCount(page: Page, text: string): Promise<number> {
  return page.getByTestId("log").locator("div", { hasText: text }).count();
}
