import { create } from "zustand";
import { isAcceptedImage, loadImageFile } from "../lib/image";
import * as ipc from "../lib/ipc";
import type { Traced, TraceSettings } from "../lib/ipc";

export type Step = "import" | "trace" | "operations" | "machine";

export interface SourceImage {
  name: string;
  url: string;
  width: number;
  height: number;
  bytes: number;
}

export const DEFAULT_TRACE_SETTINGS: TraceSettings = {
  mode: "outline",
  preprocess: {
    brightness: 0,
    contrast: 0,
    blurSigma: 0,
    threshold: { mode: "otsu" },
    invert: false,
  },
  speckle: 4,
  curves: true,
  cornerThreshold: 60,
  colorPrecision: 6,
  layerDifference: 16,
};

interface TraceState {
  settings: TraceSettings;
  result: Traced | null;
  /** True while the result on screen comes from the low-resolution preview image. */
  refining: boolean;
  busy: boolean;
  error: string | null;
}

interface ProjectState {
  step: Step;
  image: SourceImage | null;
  /** Work area in millimetres. */
  bed: { width: number; height: number };
  /** Physical width of the artwork in millimetres; height follows the aspect ratio. */
  widthMm: number;
  showSource: boolean;
  error: string | null;
  trace: TraceState;
  /** Path of the last exported SVG, for confirmation in the UI. */
  exportedTo: string | null;
  setStep: (step: Step) => void;
  importFile: (file: File) => Promise<void>;
  clearImage: () => void;
  setWidthMm: (widthMm: number) => void;
  setShowSource: (show: boolean) => void;
  updateTrace: (patch: Partial<TraceSettings>) => void;
  updatePreprocess: (patch: Partial<TraceSettings["preprocess"]>) => void;
  exportSvg: () => Promise<void>;
}

const EMPTY_TRACE: TraceState = {
  settings: DEFAULT_TRACE_SETTINGS,
  result: null,
  refining: false,
  busy: false,
  error: null,
};

const PREVIEW_DELAY_MS = 120;
const REFINE_DELAY_MS = 500;

let traceTimer: ReturnType<typeof setTimeout> | undefined;
// Bumped on every request so that a slow, outdated trace cannot overwrite a newer one.
let traceGeneration = 0;

function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export const useProject = create<ProjectState>((set, get) => {
  async function runTrace(preview: boolean) {
    const generation = ++traceGeneration;
    const patchTrace = (patch: Partial<TraceState>) =>
      set((s) => ({ trace: { ...s.trace, ...patch } }));

    patchTrace({ busy: true });
    try {
      const result = await ipc.traceImage(get().trace.settings, preview);
      if (generation !== traceGeneration) return;
      patchTrace({ result, refining: preview, busy: preview, error: null });
      if (preview) {
        traceTimer = setTimeout(() => void runTrace(false), REFINE_DELAY_MS);
      }
    } catch (e) {
      if (generation !== traceGeneration) return;
      patchTrace({ busy: false, refining: false, error: message(e) });
    }
  }

  function scheduleTrace() {
    if (!get().image || !ipc.isTauri()) return;
    clearTimeout(traceTimer);
    traceGeneration++;
    traceTimer = setTimeout(() => void runTrace(true), PREVIEW_DELAY_MS);
  }

  function cancelTrace() {
    clearTimeout(traceTimer);
    traceGeneration++;
  }

  return {
    step: "import",
    image: null,
    bed: { width: 400, height: 400 },
    widthMm: 100,
    showSource: true,
    error: null,
    trace: EMPTY_TRACE,
    exportedTo: null,

    setStep: (step) => set({ step }),

    importFile: async (file) => {
      if (!isAcceptedImage(file)) {
        set({ error: `${file.name} is not a supported image (PNG, JPEG, WebP, BMP or GIF).` });
        return;
      }
      try {
        const image = await loadImageFile(file);
        if (ipc.isTauri()) {
          try {
            await ipc.loadImage(new Uint8Array(await file.arrayBuffer()));
          } catch (e) {
            URL.revokeObjectURL(image.url);
            throw e;
          }
        }
        cancelTrace();
        const previous = get().image;
        if (previous) URL.revokeObjectURL(previous.url);

        // Start at half the bed, whichever side is the tighter fit.
        const { bed, trace } = get();
        const scale = Math.min((bed.width * 0.5) / image.width, (bed.height * 0.5) / image.height);
        set({
          image,
          widthMm: Math.round(image.width * scale),
          error: null,
          exportedTo: null,
          trace: { ...EMPTY_TRACE, settings: trace.settings },
          step: "trace",
        });
        scheduleTrace();
      } catch (e) {
        set({ error: message(e) });
      }
    },

    clearImage: () => {
      cancelTrace();
      const previous = get().image;
      if (previous) URL.revokeObjectURL(previous.url);
      if (ipc.isTauri()) void ipc.clearImage();
      set((s) => ({
        image: null,
        error: null,
        exportedTo: null,
        trace: { ...EMPTY_TRACE, settings: s.trace.settings },
      }));
    },

    setWidthMm: (widthMm) => set({ widthMm }),
    setShowSource: (showSource) => set({ showSource }),

    updateTrace: (patch) => {
      set((s) => ({ trace: { ...s.trace, settings: { ...s.trace.settings, ...patch } } }));
      scheduleTrace();
    },

    updatePreprocess: (patch) => {
      const { settings } = get().trace;
      get().updateTrace({ preprocess: { ...settings.preprocess, ...patch } });
    },

    exportSvg: async () => {
      const { image, trace, widthMm } = get();
      if (!image) return;
      const suggestedName = `${image.name.replace(/\.[^.]+$/, "")}.svg`;
      try {
        const path = await ipc.exportSvg(trace.settings, widthMm, suggestedName);
        if (path) set((s) => ({ exportedTo: path, trace: { ...s.trace, error: null } }));
      } catch (e) {
        set((s) => ({ trace: { ...s.trace, error: message(e) } }));
      }
    },
  };
});
