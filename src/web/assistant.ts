/**
 * The assistant and picture services, from a web page. The engine writes each
 * request and reads each reply, exactly as the desktop app does; the page only
 * carries them, since only it can reach the network.
 *
 * A page has no credential store, so API keys are kept in this browser's
 * storage for this site.
 */
import { engine } from "./engine";

interface Endpoint {
  protocol: string;
  baseUrl: string;
  model: string;
}

interface WebRequest {
  url: string;
  headers: [string, string][];
  body: unknown | null;
}

const keyName = (keyId: string) => `kerf.key.${keyId}`;

function savedKey(keyId: string): string | null {
  return localStorage.getItem(keyName(keyId));
}

export async function setApiKey({ keyId, key }: { keyId: string; key: string }) {
  localStorage.setItem(keyName(keyId), key.trim());
}

export async function hasApiKey({ keyId }: { keyId: string }) {
  return savedKey(keyId) !== null;
}

export async function deleteApiKey({ keyId }: { keyId: string }) {
  localStorage.removeItem(keyName(keyId));
}

const onThisComputer = (url: string) => /^https?:\/\/(localhost|127\.0\.0\.1|\[::1\])[:/]/i.test(url);

/** Makes a request the engine wrote, and resolves to the reply's JSON. */
async function send(request: WebRequest): Promise<unknown> {
  const headers = new Headers(request.headers);
  if (request.body !== null) headers.set("Content-Type", "application/json");
  // Anthropic only answers a web page that says it knows the key is in a browser.
  if (headers.has("x-api-key")) headers.set("anthropic-dangerous-direct-browser-access", "true");
  let response: Response;
  try {
    response = await fetch(request.url, {
      method: request.body === null ? "GET" : "POST",
      headers,
      body: request.body === null ? undefined : JSON.stringify(request.body),
    });
  } catch {
    // A browser does not say why: unreachable, or reachable but not accepting requests from web pages.
    const why = onThisComputer(request.url)
      ? `it is not running, or it does not accept requests from web pages. For Ollama, set OLLAMA_ORIGINS to ${location.origin} and start it again`
      : "it did not answer, or it does not accept requests from web pages";
    throw await engine<string>("could_not_reach", { url: request.url, why });
  }
  const body: unknown = await response.json().catch(() => null);
  if (!response.ok) throw await engine<string>("describe_failure", { status: response.status, body });
  if (body === null) throw "The provider's reply could not be read.";
  return body;
}

export async function assistantSend({
  endpoint,
  keyId,
  system,
  messages,
  tools,
}: {
  endpoint: Endpoint;
  keyId: string;
  system: string;
  messages: unknown[];
  tools: unknown[];
}) {
  const request = await engine<WebRequest>("turn_request", { endpoint, key: savedKey(keyId), system, messages, tools });
  const reply = await send(request);
  return engine("turn_reply", { endpoint, reply, tools, sent: messages.length });
}

export async function assistantModels({ endpoint, keyId }: { endpoint: Endpoint; keyId: string }) {
  const request = await engine<WebRequest>("models_request", { endpoint, key: savedKey(keyId) });
  return engine("models_reply", { endpoint, reply: await send(request) });
}

const blocked = "That site does not let a web page download the picture. Save the picture to this computer and drop the file here, or copy the picture itself and press Ctrl+V.";

/** Downloads a picture at a web address, where its site allows a page to. */
export async function fetchPicture({ url }: { url: string }): Promise<ArrayBuffer> {
  let response: Response;
  try {
    response = await fetch(url);
  } catch {
    throw blocked;
  }
  if (!response.ok) throw `The picture could not be downloaded (HTTP ${response.status}).`;
  return response.arrayBuffer();
}

/** Makes a picture from a description and resolves to the image file's bytes. */
export async function generatePicture({
  endpoint,
  keyId,
  prompt,
  shape,
}: {
  endpoint: Endpoint;
  keyId: string;
  prompt: string;
  shape: string;
}): Promise<ArrayBuffer> {
  const request = await engine<WebRequest>("picture_request", { endpoint, key: savedKey(keyId), prompt, shape });
  const reply = await send(request);
  const made = await engine<{ base64?: string; url?: string }>("picture_reply", { endpoint, reply });
  if (made.url) return fetchPicture({ url: made.url });
  const text = atob(made.base64 ?? "");
  const bytes = new Uint8Array(text.length);
  for (let i = 0; i < text.length; i++) bytes[i] = text.charCodeAt(i);
  return bytes.buffer;
}