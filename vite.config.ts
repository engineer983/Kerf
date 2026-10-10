import { createReadStream, existsSync, readFileSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

const host = process.env.TAURI_DEV_HOST;

/**
 * Serves the ONNX runtime's WebAssembly files from the app itself, under
 * `/ort/`, so the depth model does not need a CDN to run. They are taken from
 * the copy that `@huggingface/transformers` depends on, so the versions match.
 */
function onnxRuntimeFiles(): Plugin {
  const files = ["ort-wasm-simd-threaded.asyncify.mjs", "ort-wasm-simd-threaded.asyncify.wasm"];
  const require = createRequire(import.meta.url);
  const transformers = require.resolve("@huggingface/transformers");
  const dist = path.dirname(createRequire(transformers).resolve("onnxruntime-web"));
  for (const file of files) {
    if (!existsSync(path.join(dist, file))) {
      throw new Error(`The ONNX runtime file ${file} was not found in ${dist}`);
    }
  }
  return {
    name: "kerf-onnx-runtime-files",
    configureServer(server) {
      server.middlewares.use((request, response, next) => {
        const file = files.find((f) => request.url?.split("?")[0] === `/ort/${f}`);
        if (!file) return next();
        response.setHeader("Content-Type", file.endsWith(".wasm") ? "application/wasm" : "text/javascript");
        createReadStream(path.join(dist, file)).pipe(response);
      });
    },
    generateBundle() {
      for (const file of files) {
        this.emitFile({ type: "asset", fileName: `ort/${file}`, source: readFileSync(path.join(dist, file)) });
      }
    },
  };
}

export default defineConfig({
  plugins: [react(), tailwindcss(), onnxRuntimeFiles()],
  // Keep Rust compiler errors visible in the terminal.
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    watch: { ignored: ["**/src-tauri/**", "**/crates/**", "**/target/**"] },
  },
});
