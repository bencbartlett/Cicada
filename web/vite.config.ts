/// <reference types="vitest/config" />
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// Dev flow per doc 15 stage 5: Vite dev server proxying to `cicada serve`;
// release embeds the built SPA in the cicada binary. The proxy target is
// wired here when the server exists.
export default defineConfig({
  plugins: [react()],
  test: {
    environment: "node",
  },
});
