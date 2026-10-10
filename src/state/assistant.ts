import { create } from "zustand";
import { runTool, SYSTEM_PROMPT, TOOLS } from "../lib/assistantTools";
import * as ipc from "../lib/ipc";
import { findProvider, type Protocol } from "../lib/providers";

const SETTINGS_KEY = "kerf.assistant";
/** The most times in a row the model may call tools before it must answer. */
const MAX_ROUNDS = 12;

/** What the person sees in the conversation. */
export type Entry =
  | { kind: "user"; text: string }
  | { kind: "assistant"; text: string }
  /** Something the assistant did to the project. */
  | { kind: "action"; text: string; failed: boolean }
  | { kind: "notice"; text: string };

interface Settings {
  providerId: string;
  /** The model chosen for each provider. */
  models: Record<string, string>;
  /** The address used for each provider, where it differs from the preset. */
  baseUrls: Record<string, string>;
}

function loadSettings(): Settings {
  try {
    const stored = JSON.parse(localStorage.getItem(SETTINGS_KEY) ?? "null");
    if (stored && typeof stored.providerId === "string") {
      return { providerId: stored.providerId, models: stored.models ?? {}, baseUrls: stored.baseUrls ?? {} };
    }
  } catch {
    // Unreadable; start from the defaults.
  }
  return { providerId: "anthropic", models: {}, baseUrls: {} };
}

interface AssistantState extends Settings {
  entries: Entry[];
  /** The conversation in the provider's own message format. */
  history: unknown[];
  busy: boolean;
  /** What the assistant is doing while busy, for display. */
  activity: string | null;
  hasKey: boolean;
  /** Models the provider offers, once loaded. */
  available: ipc.ModelInfo[];
  settingsError: string | null;
  setProvider: (id: string) => void;
  setModel: (model: string) => void;
  setBaseUrl: (url: string) => void;
  saveKey: (key: string) => Promise<void>;
  removeKey: () => Promise<void>;
  loadModels: () => Promise<void>;
  refreshKey: () => Promise<void>;
  send: (text: string) => Promise<void>;
  newConversation: () => void;
}

function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** A person's message, in the provider's format. Both protocols share this shape. */
const userMessage = (text: string) => ({ role: "user", content: text });

/** The results of the model's tool calls, in the provider's format. */
function toolResults(protocol: Protocol, results: { id: string; content: string; isError: boolean }[]): unknown[] {
  if (protocol === "anthropic") {
    // All results go back together in one message.
    return [
      {
        role: "user",
        content: results.map((r) => ({
          type: "tool_result",
          tool_use_id: r.id,
          content: r.content,
          ...(r.isError && { is_error: true }),
        })),
      },
    ];
  }
  return results.map((r) => ({ role: "tool", tool_call_id: r.id, content: r.isError ? `Error: ${r.content}` : r.content }));
}

export const useAssistant = create<AssistantState>((set, get) => {
  const persist = () => {
    const { providerId, models, baseUrls } = get();
    try {
      localStorage.setItem(SETTINGS_KEY, JSON.stringify({ providerId, models, baseUrls }));
    } catch {
      // Not persisted; the choice still applies for this session.
    }
  };

  const endpoint = (): ipc.Endpoint => {
    const { providerId, models, baseUrls } = get();
    const preset = findProvider(providerId);
    return {
      protocol: preset.protocol,
      baseUrl: (baseUrls[providerId] ?? preset.baseUrl).trim(),
      model: (models[providerId] ?? preset.defaultModel).trim(),
    };
  };

  return {
    ...loadSettings(),
    entries: [],
    history: [],
    busy: false,
    activity: null,
    hasKey: false,
    available: [],
    settingsError: null,

    // A conversation belongs to the model it was had with: each provider keeps
    // its own records in the history, which another cannot read.
    setProvider: (providerId) => {
      set({ providerId, available: [], settingsError: null, entries: [], history: [] });
      persist();
      void get().refreshKey();
    },
    setModel: (model) => {
      set((s) => ({ models: { ...s.models, [s.providerId]: model }, entries: [], history: [] }));
      persist();
    },
    setBaseUrl: (url) => {
      set((s) => ({ baseUrls: { ...s.baseUrls, [s.providerId]: url }, available: [] }));
      persist();
    },

    refreshKey: async () => {
      if (!ipc.isTauri()) return;
      try {
        set({ hasKey: await ipc.hasApiKey(get().providerId) });
      } catch (e) {
        set({ settingsError: message(e) });
      }
    },
    saveKey: async (key) => {
      try {
        await ipc.setApiKey(get().providerId, key);
        set({ hasKey: true, settingsError: null });
      } catch (e) {
        set({ settingsError: message(e) });
      }
    },
    removeKey: async () => {
      try {
        await ipc.deleteApiKey(get().providerId);
        set({ hasKey: false, settingsError: null });
      } catch (e) {
        set({ settingsError: message(e) });
      }
    },
    loadModels: async () => {
      try {
        set({ available: await ipc.assistantModels(endpoint(), get().providerId), settingsError: null });
      } catch (e) {
        set({ available: [], settingsError: message(e) });
      }
    },

    newConversation: () => set({ entries: [], history: [] }),

    send: async (text) => {
      const question = text.trim();
      if (!question || get().busy) return;
      const target = endpoint();
      const show = (entry: Entry) => set((s) => ({ entries: [...s.entries, entry] }));
      show({ kind: "user", text: question });
      if (!target.model) {
        show({ kind: "notice", text: "Choose a model in the assistant's settings first." });
        return;
      }

      const history = [...get().history, userMessage(question)];
      set({ busy: true, activity: "Thinking…", history });
      try {
        for (let round = 0; round < MAX_ROUNDS; round++) {
          const turn = await ipc.assistantSend(target, get().providerId, SYSTEM_PROMPT, history, TOOLS);
          // The reply goes into the history exactly as the provider sent it.
          history.push(turn.message);
          set({ history: [...history] });
          if (turn.text.trim()) show({ kind: "assistant", text: turn.text.trim() });

          if (turn.stop.type === "refused") {
            show({ kind: "notice", text: `The model declined to answer: ${turn.stop.explanation}` });
            break;
          }
          if (turn.stop.type === "truncated") {
            show({ kind: "notice", text: "The reply was cut off for length. Ask it to continue if you need the rest." });
            break;
          }
          if (turn.toolCalls.length === 0) break;

          const results = [];
          for (const call of turn.toolCalls) {
            set({ activity: "Working on the project…" });
            const outcome = await runTool(call.name, call.input);
            // Reading the project is not worth a line in the conversation.
            if (call.name !== "get_project" || outcome.isError) {
              show({ kind: "action", text: outcome.summary, failed: outcome.isError });
            }
            results.push({ id: call.id, content: outcome.content, isError: outcome.isError });
          }
          history.push(...toolResults(target.protocol, results));
          set({ history: [...history], activity: "Thinking…" });
          if (round === MAX_ROUNDS - 1) {
            show({ kind: "notice", text: "The assistant stopped after many steps in a row. Ask it to carry on if it was not finished." });
          }
        }
      } catch (e) {
        show({ kind: "notice", text: message(e) });
        // A question that got no reply at all is dropped from the history, so
        // that asking again does not send it twice.
        const last = history.at(-1) as { role?: string; content?: unknown } | undefined;
        if (last?.role === "user" && last.content === question) set({ history: history.slice(0, -1) });
      } finally {
        set({ busy: false, activity: null });
      }
    },
  };
});
