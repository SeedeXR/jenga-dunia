import { defineConfig } from "@playwright/test";

// UI-driven end-to-end tests: the real frontend in Chromium, with Tauri's `invoke` bridged to the
// real Rust core through `jd-mcp` (see e2e/bridge.ts). Runs in CI without a display.
export default defineConfig({
  testDir: "./e2e",
  timeout: 90_000,
  retries: 0,
  reporter: [["list"]],
  use: {
    baseURL: "http://localhost:5173",
    viewport: { width: 1440, height: 900 },
    colorScheme: "dark",
    screenshot: "only-on-failure",
    trace: "retain-on-failure",
  },
  webServer: {
    command: "pnpm dev",
    url: "http://localhost:5173",
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  },
});
