/// <reference types="vitest/config" />
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { readFileSync } from "node:fs";
import { defineConfig } from "vite";

// Tauri dev server: fixed port, no auto-switch, so `tauri dev` always finds it.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  // One version everywhere: the repo's VERSION file (checked against the manifests by scripts/check-version.mjs).
  define: { __JD_VERSION__: JSON.stringify(readFileSync(new URL("../../VERSION", import.meta.url), "utf8").trim()) },
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: { target: ["es2022", "safari17"], sourcemap: true },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test/setup.ts"],
    include: ["src/**/*.test.{ts,tsx}"],
  },
});
