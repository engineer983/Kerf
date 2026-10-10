import { create } from "zustand";
import * as ipc from "../lib/ipc";

// Where the assistant gets pictures it cannot draw itself: an image model
// (hosted, or on this computer) and a search of pictures free to reuse.

const SETTINGS_KEY = "kerf.pictures";

export interface PictureSource {
  id: string;
  label: string;
  protocol: ipc.ImageEndpoint["protocol"];
  baseUrl: string;
  /** Whether the address can be changed. Hosted services have one fixed address. */
  addressEditable: boolean;
  defaultModel: string;
  /** The name the service's API key is saved under, if it can take one. */
  keyId: string | null;
  needsKey: boolean;
  note: string;
}

export const OFF = "off";

export const PICTURE_SOURCES: PictureSource[] = [
  {
    id: "openai",
    label: "OpenAI (hosted)",
    protocol: "openAi",
    baseUrl: "https://api.openai.com/v1",
    addressEditable: false,
    defaultModel: "gpt-image-1",
    // The same key serves OpenAI's chat models and its image models.
    keyId: "openai",
    needsKey: true,
    note: "Needs an API key from platform.openai.com. Every picture made is billed to your OpenAI account, and OpenAI may ask you to verify your organisation before its image models work.",
  },
  {
    id: "webui",
    label: "Stable Diffusion web UI (on this computer)",
    protocol: "webUi",
    baseUrl: "http://127.0.0.1:7860",
    addressEditable: true,
    defaultModel: "",
    keyId: null,
    needsKey: false,
    note: "Free and private: pictures are made on this computer. Install AUTOMATIC1111 or Forge with a model, and start it with the --api option. Needs a capable graphics card to be quick. Leave the model empty to use the one the web UI has loaded.",
  },
  {
    id: "custom",
    label: "Other OpenAI-compatible image server",
    protocol: "openAi",
    baseUrl: "",
    addressEditable: true,
    defaultModel: "",
    keyId: "image-custom",
    needsKey: false,
    note: "Any server with an OpenAI-style images endpoint, such as LocalAI. Enter its address ending in /v1, and a key if it needs one.",
  },
];

export const findPictureSource = (id: string) => PICTURE_SOURCES.find((s) => s.id === id) ?? null;

interface Settings {
  /** A source's id, or "off". */
  sourceId: string;
  models: Record<string, string>;
  baseUrls: Record<string, string>;
  /** Whether the assistant may search the web for pictures. */
  webSearch: boolean;
}

function loadSettings(): Settings {
  try {
    const stored = JSON.parse(localStorage.getItem(SETTINGS_KEY) ?? "null");
    if (stored && typeof stored.sourceId === "string") {
      return {
        sourceId: stored.sourceId,
        models: stored.models ?? {},
        baseUrls: stored.baseUrls ?? {},
        webSearch: stored.webSearch !== false,
      };
    }
  } catch {
    // Unreadable; start from the defaults.
  }
  return { sourceId: OFF, models: {}, baseUrls: {}, webSearch: true };
}

interface PicturesState extends Settings {
  hasKey: boolean;
  error: string | null;
  /** The pictures the last search found, which the assistant refers to by number. */
  found: ipc.FoundPicture[];
  setSource: (id: string) => void;
  setModel: (model: string) => void;
  setBaseUrl: (url: string) => void;
  setWebSearch: (on: boolean) => void;
  saveKey: (key: string) => Promise<void>;
  removeKey: () => Promise<void>;
  refreshKey: () => Promise<void>;
  setFound: (found: ipc.FoundPicture[]) => void;
}

function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export const usePictures = create<PicturesState>((set, get) => {
  const persist = () => {
    const { sourceId, models, baseUrls, webSearch } = get();
    try {
      localStorage.setItem(SETTINGS_KEY, JSON.stringify({ sourceId, models, baseUrls, webSearch }));
    } catch {
      // Not persisted; the choice still applies for this session.
    }
  };
  const keyId = () => findPictureSource(get().sourceId)?.keyId ?? null;

  return {
    ...loadSettings(),
    hasKey: false,
    error: null,
    found: [],

    setSource: (sourceId) => {
      set({ sourceId, error: null, hasKey: false });
      persist();
      void get().refreshKey();
    },
    setModel: (model) => {
      set((s) => ({ models: { ...s.models, [s.sourceId]: model } }));
      persist();
    },
    setBaseUrl: (url) => {
      set((s) => ({ baseUrls: { ...s.baseUrls, [s.sourceId]: url } }));
      persist();
    },
    setWebSearch: (webSearch) => {
      set({ webSearch });
      persist();
    },

    refreshKey: async () => {
      const id = keyId();
      if (!id || !ipc.isTauri()) return;
      try {
        set({ hasKey: await ipc.hasApiKey(id) });
      } catch (e) {
        set({ error: message(e) });
      }
    },
    saveKey: async (key) => {
      const id = keyId();
      if (!id) return;
      // No image service here takes an Anthropic key, and a saved key is sent
      // to the service on the next request.
      if (key.trim().startsWith("sk-ant-")) {
        set({ error: "That looks like an Anthropic (Claude) key, which cannot make pictures. It was not saved." });
        return;
      }
      try {
        await ipc.setApiKey(id, key);
        set({ hasKey: true, error: null });
      } catch (e) {
        set({ error: message(e) });
      }
    },
    removeKey: async () => {
      const id = keyId();
      if (!id) return;
      try {
        await ipc.deleteApiKey(id);
        set({ hasKey: false, error: null });
      } catch (e) {
        set({ error: message(e) });
      }
    },

    setFound: (found) => set({ found }),
  };
});

/** Where to ask for a picture, or the reason one cannot be asked for yet. */
export function pictureEndpoint(): { endpoint: ipc.ImageEndpoint; keyId: string } | { problem: string } {
  const s = usePictures.getState();
  const source = findPictureSource(s.sourceId);
  if (!source) return { problem: "Making pictures is switched off." };
  const baseUrl = ((source.addressEditable ? s.baseUrls[source.id] : undefined) ?? source.baseUrl).trim();
  if (!baseUrl) return { problem: "The image server's address has not been entered." };
  return {
    endpoint: { protocol: source.protocol, baseUrl, model: (s.models[source.id] ?? source.defaultModel).trim() },
    // A source with no key is asked under a name nothing is saved under.
    keyId: source.keyId ?? "image-none",
  };
}
