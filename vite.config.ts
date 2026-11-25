import { readFileSync } from "node:fs";
import { defineConfig } from "vite";

const pkg = JSON.parse(readFileSync(new URL("./package.json", import.meta.url), "utf8")) as { version: string; buildName?: string };

// Tauri expects a fixed dev port and must not have its console cleared.
export default defineConfig({
  clearScreen: false,
  // The installed build's name, compiled into the app it ships in (see views/shell.ts).
  define: { __CLAWS_BUILD__: JSON.stringify(pkg.buildName ?? pkg.version) },
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    target: "es2022",
    outDir: "dist",
    emptyOutDir: true,
  },
});
