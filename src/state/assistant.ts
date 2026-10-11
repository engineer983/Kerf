import { create } from "zustand";
import { runTool, SYSTEM_PROMPT, toolActivity, TOOLS } from "../lib/assistantTools";
import * as ipc from "../lib/ipc";
import { findProvider, type Protocol } from "../lib/providers";
import { useProject } from "./project";

// Version 2: an address is only remembered for providers whose address can be
// changed. Settings saved under the first key are discarded, since they could
// hold an address for a hosted provider, which nothing in the interface could
// then correct.
const SETTINGS_KEY = "kerf.assistant.v2";
const OLD_SETTINGS_KEY = "kerf.assistant";
/** The most times in a row the model may call tools before it must answer. */
const MAX_ROUNDS = 12;

/** What the person sees in the conversation. */
export type Entry =
  | { kind: "user"; text: string }
  | { kind: "assistant"; text: string }
  /** Something the assistant did to the project. */
  | { kind: "action"; text: string; failed: boolean }
  /** Pictures a search found, to choose from. */
  | { kind: "pictures"; pictures: ipc.FoundPicture[] }
  | { kind: "notice"; text: string };

interface Settings {
  providerId: string;
  /** The model chosen for each provider. */
  models: Record<string, string>;
  /** The address used for each provider, where it differs from the preset. */
  baseUrls: Record<string, string>;
  /** Send a small copy of the picture with the first message, so the model can see it. */
  sharePicture: boolean;
}

function loadSettings(): Settings {
  try {
    localStorage.removeItem(OLD_SETTINGS_KEY);
    const stored = JSON.parse(localStorage.getItem(SETTINGS_KEY) ?? "null");
    if (stored && typeof stored.providerId === "string") {
      return {
        providerId: stored.providerId,
        models: stored.models ?? {},
        baseUrls: stored.baseUrls ?? {},
        sharePicture: stored.sharePicture === true,
      };
    }
  } catch {
    // Unreadable; start from the defaults.
  }
  return { providerId: "anthropic", models: {}, baseUrls: {}, sharePicture: false };
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
  setSharePicture: (share: boolean) => void;
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

/** Longest side, in pixels, of the copy of the picture a model is shown. */
const SHARED_PICTURE_PX = 1024;

/** The project's picture as a small JPEG, base64-encoded, or null if there is none. */
async function pictureForModel(): Promise<string | null> {
  const { image } = useProject.getState();
  if (!image) return null;
  const picture = new Image();
  picture.src = image.url;
  await picture.decode();
  const scale = Math.min(1, SHARED_PICTURE_PX / Math.max(picture.naturalWidth, picture.naturalHeight));
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(picture.naturalWidth * scale));
  canvas.height = Math.max(1, Math.round(picture.naturalHeight * scale));
  const ctx = canvas.getContext("2d")!;
  // Transparent areas become white, as they are treated everywhere else.
  ctx.fillStyle = "#fff";
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  ctx.drawImage(picture, 0, 0, canvas.width, canvas.height);
  return canvas.toDataURL("image/jpeg", 0.85).split(",")[1];
}

/** A person's message with the picture attached, in the provider's format. */
function userMessageWithPicture(protocol: Protocol, text: string, jpeg: string) {
  const note = `${text}\n\n(The picture in the project is attached.)`;
  return protocol === "anthropic"
    ? {
        role: "user",
        content: [
          { type: "image", source: { type: "base64", media_type: "image/jpeg", data: jpeg } },
          { type: "text", text: note },
        ],
      }
    : {
        role: "user",
        content: [
          { type: "image_url", image_url: { url: `data:image/jpeg;base64,${jpeg}` } },
          { type: "text", text: note },
        ],
      };
}

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
    const { providerId, models, baseUrls, sharePicture } = get();
    try {
      localStorage.setItem(SETTINGS_KEY, JSON.stringify({ providerId, models, baseUrls, sharePicture }));
    } catch {
      // Not persisted; the choice still applies for this session.
    }
  };

  const endpoint = (): ipc.Endpoint => {
    const { providerId, models, baseUrls } = get();
    const preset = findProvider(providerId);
    return {
      protocol: preset.protocol,
      // A hosted provider is always reached at its own address.
      baseUrl: ((preset.addressEditable ? baseUrls[providerId] : undefined) ?? preset.baseUrl).trim(),
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

    setSharePicture: (sharePicture) => {
      set({ sharePicture });
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
      // A key pasted under the wrong provider would be sent to that provider
      // on the next request, so an obvious mismatch is refused here.
      const isAnthropicKey = key.trim().startsWith("sk-ant-");
      const { providerId } = get();
      if (isAnthropicKey && providerId !== "anthropic") {
        set({
          settingsError:
            "That looks like an Anthropic (Claude) key. It was not saved. Choose Claude (Anthropic) as the model provider, then paste it there.",
        });
        return;
      }
      if (!isAnthropicKey && providerId === "anthropic") {
        set({
          settingsError:
            "That does not look like an Anthropic key, which starts with sk-ant-. It was not saved. Check the key, or choose the provider it belongs to.",
        });
        return;
      }
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

      set({ busy: true, activity: "Thinking…" });
      // The picture goes with the first message of a conversation only: the
      // model keeps it in view from there, and resending it would cost more.
      const plain: unknown = userMessage(question);
      let first = plain;
      if (get().sharePicture && get().history.length === 0) {
        try {
          const jpeg = await pictureForModel();
          if (jpeg) first = userMessageWithPicture(target.protocol, question, jpeg);
        } catch {
          // The question is still worth asking without the picture.
        }
      }
      const history = [...get().history, first];
      set({ history });
      try {
        for (let round = 0; round < MAX_ROUNDS; round++) {
          let turn: ipc.AssistantTurn;
          try {
            turn = await ipc.assistantSend(target, get().providerId, SYSTEM_PROMPT, history, TOOLS);
          } catch (e) {
            // Many models, local ones especially, cannot take pictures and
            // reject the whole request. The question is asked again without it.
            if (round !== 0 || first === plain) throw e;
            first = plain;
            history[history.length - 1] = plain;
            turn = await ipc.assistantSend(target, get().providerId, SYSTEM_PROMPT, history, TOOLS);
            show({
              kind: "notice",
              text: "This model could not take the picture, so your message was sent without it. To stop this notice, untick \"Let the assistant see the picture\" in the assistant's settings.",
            });
          }
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
            set({ activity: toolActivity(call.name) });
            const outcome = await runTool(call.name, call.input);
            // Reading the project is not worth a line in the conversation.
            if (call.name !== "get_project" || outcome.isError) {
              show({ kind: "action", text: outcome.summary, failed: outcome.isError });
            }
            if (outcome.pictures?.length) show({ kind: "pictures", pictures: outcome.pictures });
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
        const last = history.at(-1);
        if (last === first) set({ history: history.slice(0, -1) });
      } finally {
        set({ busy: false, activity: null });
      }
    },
  };
});
