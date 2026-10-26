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
    // A 2048² u16 preview is 8 MB (10.7 MB as base64): above the SDK's 10 MiB default read buffer.
    maxBufferSize: 64 * 1024 * 1024,
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
  // Under load a re-render between the two clicks drops React Flow's pending connection (seen as a
  // missing `graph.connect` in the suite only): click the pair again, like `selectNode` retries.
  const before = await logCount(page, "graph.connect: ok");
  for (let i = 0; ; i++) {
    await src.click({ force: true });
    await dst.click({ force: true });
    try {
      await expect.poll(() => logCount(page, "graph.connect: ok"), { timeout: 2500 }).toBeGreaterThan(before);
      return;
    } catch (e) {
      if (i >= 3) throw e;
    }
  }
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

/** Select a node: click until React Flow marks it `selected` (under load a click lands while the graph re-renders and is lost). */
export async function selectNode(node: Locator): Promise<void> {
  for (let i = 0; ; i++) {
    await node.click({ force: true });
    try {
      await expect(node).toHaveClass(/selected/, { timeout: 2000 });
      return;
    } catch (e) {
      if (i >= 4) throw e;
    }
  }
}

/** How many lines of the op log contain `text` (assert on a count, not on text that an earlier op already logged). */
/** The newest log line matching `text`, as its session sequence number (0: none): it rises with
 *  each new match even once the 200-line log is full and old matches scroll out, which a count of
 *  the visible lines does not (a connect could then add one match as another dropped off). */
export async function logCount(page: Page, text: string | RegExp): Promise<number> {
  const seqs = await page.getByTestId("log").locator("div", { hasText: text }).evaluateAll((els) => els.map((e) => Number(e.getAttribute("data-seq"))));
  return Math.max(0, ...seqs);
}

const EVALUATED = /graph\.evaluate: ok|terrain\.generate: ok/;

/**
 * Click Generate or Evaluate and wait for that evaluation to finish. The live preview can show
 * `layer-stats` and a triangle count before the explicit run returns, and while it runs the app
 * is busy: every gated button is disabled, and a click on a disabled button is lost (seen as
 * suite-only "flakes" in graph, hydrology and paint). Counting the op's own log line is exact.
 */
export async function evaluateAndWait(page: Page, button: "generate" | "graph-evaluate", timeout = 120_000): Promise<void> {
  const before = await logCount(page, EVALUATED);
  await page.getByTestId(button).click();
  await expect.poll(() => logCount(page, EVALUATED), { timeout }).toBeGreaterThan(before);
}
