/**
 * Estimating depth from a single photograph, so that a relief can be carved
 * from the scene's shape instead of its brightness.
 *
 * The model (Depth Anything V2 Small, Apache-2.0) runs on this computer. It is
 * downloaded from Hugging Face the first time and kept in the browser cache;
 * the photograph itself is never sent anywhere.
 */

const MODEL = "onnx-community/depth-anything-v2-small";

/** The ONNX runtime files, served by the app itself (see `vite.config.ts`). */
const RUNTIME = {
  mjs: "/ort/ort-wasm-simd-threaded.asyncify.mjs",
  wasm: "/ort/ort-wasm-simd-threaded.asyncify.wasm",
};

type Estimator = (image: string) => Promise<{ depth: { toBlob: (type?: string) => Promise<Blob> } }>;

let estimator: Promise<Estimator> | null = null;

function load(onProgress: (text: string) => void): Promise<Estimator> {
  estimator ??= (async () => {
    // Loaded on demand: the library is large and most sessions never need it.
    const { env, pipeline } = await import("@huggingface/transformers");
    env.allowLocalModels = false;
    const wasm = env.backends.onnx.wasm;
    if (wasm) {
      wasm.wasmPaths = {
        mjs: new URL(RUNTIME.mjs, location.href).href,
        wasm: new URL(RUNTIME.wasm, location.href).href,
      };
    }
    const loaded = await pipeline("depth-estimation", MODEL, {
      progress_callback: (event: { status?: string; progress?: number; file?: string }) => {
        if (event.status === "progress" && event.file?.endsWith(".onnx") && event.progress !== undefined) {
          onProgress(`Downloading the depth model… ${Math.round(event.progress)}%`);
        }
      },
    });
    return loaded as unknown as Estimator;
  })();
  // A failed load (no network, say) must not be remembered as the result.
  estimator.catch(() => (estimator = null));
  return estimator;
}

/**
 * Returns a greyscale PNG the size of the image, white where the scene is
 * nearest the camera and black where it is furthest.
 */
export async function estimateDepth(imageUrl: string, onProgress: (text: string) => void): Promise<Blob> {
  onProgress("Loading the depth model…");
  const run = await load(onProgress);
  onProgress("Estimating depth…");
  const { depth } = await run(imageUrl);
  return depth.toBlob("image/png");
}
