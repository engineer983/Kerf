/** How requests are shaped: Anthropic's Messages API, or OpenAI-style chat completions. */
export type Protocol = "anthropic" | "openAi";

export interface ProviderPreset {
  id: string;
  label: string;
  protocol: Protocol;
  baseUrl: string;
  /** Suggested model; empty where the choice is the user's, from the provider's own list. */
  defaultModel: string;
  /** Whether requests fail without an API key. */
  needsKey: boolean;
  note: string;
}

export const PROVIDERS: ProviderPreset[] = [
  {
    id: "anthropic",
    label: "Claude (Anthropic)",
    protocol: "anthropic",
    baseUrl: "https://api.anthropic.com",
    defaultModel: "claude-opus-5-5",
    needsKey: true,
    note: "Hosted. Needs an API key from console.anthropic.com; usage is billed to your account.",
  },
  {
    id: "openai",
    label: "OpenAI",
    protocol: "openAi",
    baseUrl: "https://api.openai.com/v1",
    defaultModel: "",
    needsKey: true,
    note: "Hosted. Needs an API key from platform.openai.com; usage is billed to your account. Load the model list and pick one.",
  },
  {
    id: "ollama",
    label: "Ollama (on this computer)",
    protocol: "openAi",
    baseUrl: "http://localhost:11434/v1",
    defaultModel: "",
    needsKey: false,
    note: "Free and private: the model runs on this computer. Install Ollama, pull a model that supports tools, then load the model list.",
  },
  {
    id: "custom",
    label: "Other OpenAI-compatible server",
    protocol: "openAi",
    baseUrl: "",
    defaultModel: "",
    needsKey: false,
    note: "Any server that speaks the OpenAI chat API, such as LM Studio or OpenRouter. Enter its address, and a key if it needs one.",
  },
];

export const findProvider = (id: string) => PROVIDERS.find((p) => p.id === id) ?? PROVIDERS[0];
