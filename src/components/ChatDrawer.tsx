import { useEffect, useRef, useState } from "react";
import { Check, Settings, SquarePen, TriangleAlert, X } from "lucide-react";
import { isTauri } from "../lib/ipc";
import { findProvider, PROVIDERS } from "../lib/providers";
import { useAssistant, type Entry } from "../state/assistant";
import { useUi } from "../state/ui";

const field = "w-full rounded-md border border-line bg-raised px-2 py-1.5 select-text";
const quiet = "rounded-lg border border-line px-3 py-1.5 transition-colors hover:text-fg disabled:opacity-40";
const iconButton = "rounded p-1 text-muted hover:text-fg";

const SUGGESTIONS = [
  "What would you change to get more detail?",
  "This job takes too long. Make it faster.",
  "Explain what each of my settings does.",
];

export function ChatDrawer() {
  const toggleChat = useUi((s) => s.toggleChat);
  const providerId = useAssistant((s) => s.providerId);
  const model = useAssistant((s) => s.models[s.providerId] ?? findProvider(s.providerId).defaultModel);
  const hasKey = useAssistant((s) => s.hasKey);
  const entries = useAssistant((s) => s.entries);
  const preset = findProvider(providerId);
  // Open on the settings until the assistant has what it needs to answer.
  const ready = Boolean(model) && (!preset.needsKey || hasKey);
  const [showSettings, setShowSettings] = useState(false);

  useEffect(() => {
    void useAssistant.getState().refreshKey();
  }, []);

  // Once the settings have opened because something was missing, they stay
  // open until dismissed, rather than vanishing the moment a key is saved.
  useEffect(() => {
    if (!ready) setShowSettings(true);
  }, [ready]);

  return (
    <aside className="flex w-[360px] shrink-0 flex-col border-l border-line bg-surface">
      <div className="flex items-center gap-1 border-b border-line px-5 py-3">
        <h2 className="text-[13px] font-semibold">Assistant</h2>
        <span className="ml-2 truncate text-[11px] text-muted">{model || "no model chosen"}</span>
        <button
          onClick={() => useAssistant.getState().newConversation()}
          disabled={entries.length === 0}
          aria-label="Start a new conversation"
          className={`${iconButton} ml-auto disabled:opacity-30`}
        >
          <SquarePen size={14} />
        </button>
        <button
          onClick={() => setShowSettings(!showSettings)}
          aria-label="Assistant settings"
          aria-pressed={showSettings}
          className={`${iconButton} ${showSettings ? "text-accent" : ""}`}
        >
          <Settings size={14} />
        </button>
        <button onClick={toggleChat} aria-label="Close assistant" className={iconButton}>
          <X size={14} />
        </button>
      </div>
      {!isTauri() ? (
        <p className="p-5 text-muted">The assistant needs the desktop app.</p>
      ) : showSettings || !ready ? (
        <AssistantSettings onDone={ready ? () => setShowSettings(false) : undefined} />
      ) : (
        <Conversation />
      )}
    </aside>
  );
}

function AssistantSettings({ onDone }: { onDone?: () => void }) {
  const a = useAssistant();
  const preset = findProvider(a.providerId);
  const model = a.models[a.providerId] ?? preset.defaultModel;
  const baseUrl = (preset.addressEditable ? a.baseUrls[a.providerId] : undefined) ?? preset.baseUrl;
  const [key, setKey] = useState("");
  const [loading, setLoading] = useState(false);

  async function loadModels() {
    setLoading(true);
    await a.loadModels();
    setLoading(false);
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto p-5">
      <label className="flex flex-col gap-1">
        Model provider
        <select className={field} value={a.providerId} onChange={(e) => a.setProvider(e.target.value)}>
          {PROVIDERS.map((p) => (
            <option key={p.id} value={p.id}>
              {p.label}
            </option>
          ))}
        </select>
      </label>
      <p className="text-muted">{preset.note}</p>

      {preset.addressEditable && (
        <label className="flex flex-col gap-1">
          Server address
          <input
            className={`${field} font-mono text-[12px]`}
            placeholder="http://localhost:1234/v1"
            value={baseUrl}
            onChange={(e) => a.setBaseUrl(e.target.value)}
          />
        </label>
      )}

      {(preset.needsKey || preset.id === "custom") && (
        <div className="flex flex-col gap-2">
          <label className="flex flex-col gap-1">
            API key{preset.needsKey ? "" : " (if the server needs one)"}
            <input
              type="password"
              className={`${field} font-mono text-[12px]`}
              placeholder={a.hasKey ? "A key is saved. Type a new one to replace it." : "Paste your key"}
              value={key}
              onChange={(e) => setKey(e.target.value)}
            />
          </label>
          <div className="flex gap-2">
            <button
              className={quiet}
              disabled={!key.trim()}
              title="Save the key in this computer's credential store"
              onClick={() => {
                void a.saveKey(key);
                setKey("");
              }}
            >
              Save key
            </button>
            <button className={quiet} disabled={!a.hasKey} title="Delete the saved key from this computer" onClick={() => void a.removeKey()}>
              Remove key
            </button>
          </div>
          <p className="text-muted">
            {a.hasKey ? "A key is saved. " : ""}Keys are kept in this computer's credential store, not in
            the project or its files, and are only sent to the provider you chose.
          </p>
        </div>
      )}

      <div className="flex flex-col gap-2">
        <label className="flex flex-col gap-1">
          Model
          {a.available.length > 0 ? (
            <select className={field} value={model} onChange={(e) => a.setModel(e.target.value)}>
              {!a.available.some((m) => m.id === model) && <option value={model}>{model || "Choose a model"}</option>}
              {a.available.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.name === m.id ? m.id : `${m.name} (${m.id})`}
                </option>
              ))}
            </select>
          ) : (
            <input
              className={`${field} font-mono text-[12px]`}
              placeholder="Load the list, or type a model's name"
              value={model}
              onChange={(e) => a.setModel(e.target.value)}
            />
          )}
        </label>
        <button
          className={`${quiet} self-start`}
          disabled={loading || !baseUrl.trim() || (preset.needsKey && !a.hasKey)}
          title="Ask the provider which models it offers"
          onClick={() => void loadModels()}
        >
          {loading ? "Loading…" : "Load model list"}
        </button>
      </div>

      {a.settingsError && (
        <p role="alert" className="rounded-lg border border-accent/40 px-3 py-2 text-accent">
          {a.settingsError}
        </p>
      )}

      <p className="text-muted">
        The assistant can read and change this project's settings, and every change can be undone. It
        cannot connect to, move or start a machine. What you type, and the project's settings, are sent
        to the provider you chose; the picture itself is not.
      </p>

      {onDone && (
        <button onClick={onDone} className="rounded-lg bg-accent px-3 py-2 font-medium text-accent-fg hover:opacity-90">
          Done
        </button>
      )}
    </div>
  );
}

function Conversation() {
  const entries = useAssistant((s) => s.entries);
  const busy = useAssistant((s) => s.busy);
  const activity = useAssistant((s) => s.activity);
  const [draft, setDraft] = useState("");
  const end = useRef<HTMLDivElement>(null);

  useEffect(() => {
    end.current?.scrollIntoView({ block: "end" });
  }, [entries, busy]);

  function send(text: string) {
    void useAssistant.getState().send(text);
    setDraft("");
  }

  return (
    <>
      <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-5">
        {entries.length === 0 && (
          <div className="flex flex-col gap-3 text-muted">
            <p>Ask about your project, or ask for a change and the assistant will make it.</p>
            {SUGGESTIONS.map((suggestion) => (
              <button
                key={suggestion}
                onClick={() => send(suggestion)}
                title="Ask this"
                className="rounded-lg border border-line px-3 py-2 text-left transition-colors hover:text-fg"
              >
                {suggestion}
              </button>
            ))}
          </div>
        )}
        {entries.map((entry, i) => (
          <Bubble key={i} entry={entry} />
        ))}
        {busy && <p className="animate-pulse text-muted">{activity ?? "Thinking…"}</p>}
        <div ref={end} />
      </div>
      <form
        className="border-t border-line p-3"
        onSubmit={(e) => {
          e.preventDefault();
          send(draft);
        }}
      >
        <textarea
          aria-label="Message to the assistant"
          placeholder="Ask, or say what you want changed"
          className={`${field} h-20 resize-none`}
          value={draft}
          disabled={busy}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            // Enter sends; Shift+Enter makes a new line.
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              send(draft);
            }
          }}
        />
        <div className="mt-2 flex items-center justify-between">
          <span className="text-[11px] text-muted">Enter to send</span>
          <button
            type="submit"
            disabled={busy || !draft.trim()}
            title="Send your message to the assistant"
            className="rounded-lg bg-accent px-3 py-1.5 font-medium text-accent-fg hover:opacity-90 disabled:opacity-40"
          >
            Send
          </button>
        </div>
      </form>
    </>
  );
}

function Bubble({ entry }: { entry: Entry }) {
  switch (entry.kind) {
    case "user":
      return <p className="self-end rounded-xl bg-raised px-3 py-2 whitespace-pre-wrap select-text">{entry.text}</p>;
    case "assistant":
      return <p className="leading-relaxed whitespace-pre-wrap select-text">{entry.text}</p>;
    case "action":
      return (
        <p className={`flex items-start gap-1.5 text-[12px] ${entry.failed ? "text-accent" : "text-muted"}`}>
          {entry.failed ? <TriangleAlert size={13} className="mt-0.5 shrink-0" /> : <Check size={13} className="mt-0.5 shrink-0" />}
          {entry.text}
        </p>
      );
    case "notice":
      return (
        <p role="alert" className="rounded-lg border border-accent/40 px-3 py-2 text-accent select-text">
          {entry.text}
        </p>
      );
  }
}
