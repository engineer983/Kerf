import { create } from "zustand";

/** How the canvas shows a generated job. */
export type ViewMode = "paths" | "result" | "3d";

export const SPEEDS = [0.25, 0.5, 1, 2, 4, 8];
/** Seconds a whole job takes to play back at 1× speed. */
export const PLAYBACK_SECONDS = 24;

interface PlaybackState {
  mode: ViewMode;
  playing: boolean;
  /** 0 at the start of the job, 1 at the end. */
  progress: number;
  speed: number;
  /** Show the previous result beside the current one, with a divider to drag. */
  compare: boolean;
  toggleCompare: () => void;
  setMode: (mode: ViewMode) => void;
  setProgress: (progress: number) => void;
  setSpeed: (speed: number) => void;
  toggle: () => void;
  /** Shows the finished job, stopped. */
  finish: () => void;
}

export const usePlayback = create<PlaybackState>((set, get) => ({
  mode: "result",
  playing: false,
  progress: 1,
  speed: 1,
  compare: false,
  toggleCompare: () => set((s) => ({ compare: !s.compare, mode: "result" })),
  setMode: (mode) => set({ mode, playing: mode === "paths" ? false : get().playing }),
  setProgress: (progress) => set({ progress: Math.min(1, Math.max(0, progress)) }),
  setSpeed: (speed) => set({ speed }),
  toggle: () => {
    const { playing, progress, mode } = get();
    // Pressing play at the end starts again from the beginning.
    set({
      playing: !playing,
      progress: !playing && progress >= 1 ? 0 : progress,
      mode: mode === "paths" ? "result" : mode,
    });
  },
  finish: () => set({ playing: false, progress: 1 }),
}));
