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

export type Maker = "box" | "layers" | "text";

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
}

export const useUi = create<UiState>((set) => ({
  theme: readTheme(),
  chatOpen: false,
  maker: null,
  makerItem: null,
  help: null,
  setHelp: (help) => set({ help }),
  setMaker: (maker, item = null) => set({ maker, makerItem: maker ? item : null }),
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
