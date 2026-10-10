import { create } from "zustand";
import { blankTool, DEFAULT_TOOLS, type Tool } from "../lib/tools";

const STORAGE_KEY = "kerf.tools";
/** Counts the sets of starting bits; raised when new kinds are added. */
const STARTER_KEY = "kerf.tools.starters";
const STARTER_VERSION = 2;

function load(): Tool[] {
  try {
    const stored = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "null");
    if (Array.isArray(stored) && stored.length > 0) {
      const tools = (stored as Tool[]).map((t) => ({ ...t, taperAngle: t.taperAngle ?? null }));
      // A toolbox saved before a kind of bit existed gets that kind's starters once.
      if (Number(localStorage.getItem(STARTER_KEY) ?? 1) < STARTER_VERSION) {
        for (const starter of DEFAULT_TOOLS) {
          if (starter.kind === "taperedBall" && !tools.some((t) => t.id === starter.id)) {
            tools.push(starter);
          }
        }
      }
      return tools;
    }
  } catch {
    // Missing or unreadable; fall back to the starting set.
  }
  return DEFAULT_TOOLS;
}

function save(tools: Tool[]) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(tools));
    localStorage.setItem(STARTER_KEY, String(STARTER_VERSION));
  } catch {
    // Not persisted; the toolbox still works for this session.
  }
}

interface ToolboxState {
  tools: Tool[];
  open: boolean;
  setOpen: (open: boolean) => void;
  /** Adds a bit and returns its id. */
  add: (tool?: Partial<Tool>) => string;
  update: (id: string, patch: Partial<Tool>) => void;
  remove: (id: string) => void;
}

export const useToolbox = create<ToolboxState>((set, get) => {
  const commit = (tools: Tool[]) => {
    save(tools);
    set({ tools });
  };
  return {
    tools: load(),
    open: false,
    setOpen: (open) => set({ open }),
    add: (tool) => {
      const added = { ...blankTool(), ...tool };
      commit([...get().tools, added]);
      return added.id;
    },
    update: (id, patch) => commit(get().tools.map((t) => (t.id === id ? { ...t, ...patch } : t))),
    remove: (id) => commit(get().tools.filter((t) => t.id !== id)),
  };
});
