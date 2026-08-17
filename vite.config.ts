import { defineConfig } from "vite";

// Tauri drives the dev server, so the port is fixed and failures must be loud
// rather than silently moving to another port.
export default defineConfig({
  root: "ui",
  build: { outDir: "dist", emptyOutDir: true, target: "safari15" },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**", "**/crates/**", "**/target/**"] },
  },
});
