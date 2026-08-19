/// <reference types="vitest/config" />
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// Dev flow (doc 15 stage 5): Vite serves the SPA with HMR and proxies the
// engine routes to `cicada serve` (default port 8420; override with
// CICADA_SERVER=http://127.0.0.1:<port>). Release embeds the built SPA in
// the cicada binary (`--features embed`) or serves it via `--web-dir`.
const server =
  (globalThis as { process?: { env: Record<string, string | undefined> } }).process?.env
    .CICADA_SERVER ?? "http://127.0.0.1:8420";

export default defineConfig({
  plugins: [react()],
  server: {
    proxy: {
      "/api": server,
      "/debug": server,
      "/health": server,
      "/ws": { target: server, ws: true },
    },
  },
  build: {
    outDir: "dist",
    sourcemap: true,
    chunkSizeWarningLimit: 1500,
  },
  test: {
    environment: "node",
    exclude: ["e2e/**", "node_modules/**"],
  },
});
