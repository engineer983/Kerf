import { create } from "zustand";
import { isAcceptedImage, loadImageFile } from "../lib/image";

export type Step = "import" | "trace" | "operations" | "machine";

export interface SourceImage {
  name: string;
  url: string;
  width: number;
  height: number;
  bytes: number;
}

interface ProjectState {
  step: Step;
  image: SourceImage | null;
  /** Work area in millimetres. */
  bed: { width: number; height: number };
  error: string | null;
  setStep: (step: Step) => void;
  importFile: (file: File) => Promise<void>;
  clearImage: () => void;
}

export const useProject = create<ProjectState>((set, get) => ({
  step: "import",
  image: null,
  bed: { width: 400, height: 400 },
  error: null,
  setStep: (step) => set({ step }),
  importFile: async (file) => {
    if (!isAcceptedImage(file)) {
      set({ error: `${file.name} is not a supported image (PNG, JPEG, WebP, BMP or GIF).` });
      return;
    }
    try {
      const image = await loadImageFile(file);
      const previous = get().image;
      if (previous) URL.revokeObjectURL(previous.url);
      set({ image, error: null });
    } catch (e) {
      set({ error: e instanceof Error ? e.message : String(e) });
    }
  },
  clearImage: () => {
    const previous = get().image;
    if (previous) URL.revokeObjectURL(previous.url);
    set({ image: null, error: null });
  },
}));
