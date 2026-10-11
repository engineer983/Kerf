import { create } from "zustand";

export type ThemeChoice = "system" | "light" | "dark";

const THEME_KEY = "kerf.theme";

function readTheme(): ThemeChoice {
  try {
    const stored = localStorage.getItem(THEME_KEY);
    if (stored === "light" || stored === "dark" || stored === "system") return stored;
  } catch {
    // Storage unavailable; fall through to the default.
  }
  return "system";
}

const systemDark = window.matchMedia("(prefers-color-scheme: dark)");

function applyTheme(theme: ThemeChoice) {
  const dark = theme === "dark" || (theme === "system" && systemDark.matches);
  document.documentElement.dataset.theme = dark ? "dark" : "light";
}

export type Maker = "box" | "layers" | "text" | "background";

/** The panels beside the canvas whose width the person can set. */
export type Panel = "inspector" | "assistant";

/** Each panel's usual width and the narrowest and widest it may be dragged to, in pixels. */
export const PANEL_WIDTHS: Record<Panel, { usual: number; min: number; max: number }> = {
  inspector: { usual: 340, min: 260, max: 720 },
  assistant: { usual: 400, min: 300, max: 900 },
};

const PANELS_KEY = "kerf.panels";

function readPanels(): Record<Panel, number> {
  const widths = { inspector: PANEL_WIDTHS.inspector.usual, assistant: PANEL_WIDTHS.assistant.usual };
  try {
    const stored = JSON.parse(localStorage.getItem(PANELS_KEY) ?? "{}") as Partial<Record<Panel, unknown>>;
    for (const panel of ["inspector", "assistant"] as const) {
      const width = stored[panel];
      if (typeof width === "number" && Number.isFinite(width)) {
        widths[panel] = Math.min(PANEL_WIDTHS[panel].max, Math.max(PANEL_WIDTHS[panel].min, width));
      }
    }
  } catch {
    // Storage unavailable or unreadable; the usual widths are used.
  }
  return widths;
}

interface UiState {
  theme: ThemeChoice;
  chatOpen: boolean;
  /** Which of the makers is open, if any. */
  maker: Maker | null;
  /**
   * What the text maker is making: a new item added to the material ("new"),
   * a change to the item with this id, or with null the main artwork itself.
   */
  makerItem: number | "new" | null;
  /** The help topic being read, or null while the guide is closed. */
  help: string | null;
  setHelp: (topic: string | null) => void;
  setTheme: (theme: ThemeChoice) => void;
  toggleChat: () => void;
  setMaker: (maker: Maker | null, item?: number | "new" | null) => void;
  /** How wide each side panel is, in pixels. */
  panels: Record<Panel, number>;
  /** Sets a panel's width, kept within its limits, and remembers it; no width goes back to the usual one. */
  setPanelWidth: (panel: Panel, width?: number) => void;
}

export const useUi = create<UiState>((set, get) => ({
  theme: readTheme(),
  chatOpen: false,
  maker: null,
  makerItem: null,
  help: null,
  setHelp: (help) => set({ help }),
  setMaker: (maker, item = null) => set({ maker, makerItem: maker ? item : null }),
  panels: readPanels(),
  setPanelWidth: (panel, width) => {
    const { usual, min, max } = PANEL_WIDTHS[panel];
    const panels = { ...get().panels, [panel]: Math.round(Math.min(max, Math.max(min, width ?? usual))) };
    try {
      localStorage.setItem(PANELS_KEY, JSON.stringify(panels));
    } catch {
      // Not remembered; the width still applies for this session.
    }
    set({ panels });
  },
  setTheme: (theme) => {
    try {
      localStorage.setItem(THEME_KEY, theme);
    } catch {
      // Not persisted; the choice still applies for this session.
    }
    applyTheme(theme);
    set({ theme });
  },
  toggleChat: () => set((s) => ({ chatOpen: !s.chatOpen })),
}));

applyTheme(useUi.getState().theme);
systemDark.addEventListener("change", () => applyTheme(useUi.getState().theme));
