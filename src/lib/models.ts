/**
 * The models that run on this computer (depth, and finding a picture's
 * subject) share one library and one copy of the ONNX runtime.
 */

/**
 * The ONNX runtime files, served by the app itself (see `vite.config.ts`).
 * They sit beside the page, which in a browser need not be at the site's root.
 */
const RUNTIME = {
  mjs: "ort/ort-wasm-simd-threaded.asyncify.mjs",
  wasm: "ort/ort-wasm-simd-threaded.asyncify.wasm",
};

/** What the library reports while it fetches a model's files. */
export interface Progress {
  status?: string;
  progress?: number;
  file?: string;
}

/** Loaded on demand: the library is large and most sessions never need it. */
export async function transformers() {
  const library = await import("@huggingface/transformers");
  library.env.allowLocalModels = false;
  const wasm = library.env.backends.onnx.wasm;
  if (wasm) {
    wasm.wasmPaths = {
      mjs: new URL(RUNTIME.mjs, document.baseURI).href,
      wasm: new URL(RUNTIME.wasm, document.baseURI).href,
    };
  }
  return library;
}

/** Reports how far a model's download has got, as `text` with a percentage. */
export function downloading(text: string, onProgress: (text: string) => void) {
  return (event: Progress) => {
    if (event.status === "progress" && event.file?.endsWith(".onnx") && event.progress !== undefined) {
      onProgress(`${text} ${Math.round(event.progress)}%`);
    }
  };
}