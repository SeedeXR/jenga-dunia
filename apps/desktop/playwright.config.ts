import { defineConfig } from "@playwright/test";

// UI-driven end-to-end tests: the real frontend in Chromium, with Tauri's `invoke` bridged to the
// real Rust core through `jd-mcp` (see e2e/bridge.ts). Runs in CI without a display.
export default defineConfig({
  testDir: "./e2e",
  // CI renders with software GL: every live preview re-tessellates the viewport, so the graph
  // flow takes ~90 s there (≈3 s locally). Generous there, tight locally.
  timeout: process.env.CI ? 300_000 : 90_000,
  retries: 0,
  reporter: [["list"]],
  use: {
    // Locally, reuse the system Chrome (no 150 MB download); CI installs Playwright's Chromium.
    channel: process.env.CI ? undefined : "chrome",
    baseURL: "http://localhost:5173",
    viewport: { width: 1440, height: 900 },
    colorScheme: "dark",
    screenshot: "only-on-failure",
    trace: "retain-on-failure",
    // JD_VIDEO=1 records every test (used to produce the tutorial recordings in docs/user/videos).
    video: process.env.JD_VIDEO ? { mode: "on", size: { width: 1440, height: 900 } } : "off",
    launchOptions: { slowMo: process.env.JD_VIDEO ? 600 : 0 }, // human-paced actions in recordings
  },
  webServer: {
    command: "pnpm dev",
    url: "http://localhost:5173",
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  },
});
