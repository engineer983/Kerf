/** The page's side of the engine: sends it commands and waits for the answers. */
import type { EngineReply, EngineRequest } from "./worker";

let worker: Worker | null = null;
let lastId = 0;
const waiting = new Map<number, { resolve: (reply: EngineReply) => void; reject: (reason: string) => void }>();

function start(): Worker {
  const started = new Worker(new URL("./worker.ts", import.meta.url), { type: "module" });
  started.onmessage = (event: MessageEvent<EngineReply>) => {
    const pending = waiting.get(event.data.id);
    waiting.delete(event.data.id);
    if (!pending) return;
    if ("error" in event.data) pending.reject(event.data.error);
    else pending.resolve(event.data);
  };
  // The engine itself could not be loaded or has stopped: nothing asked of it will be answered.
  started.onerror = () => {
    for (const pending of waiting.values()) {
      pending.reject("Kerf's engine could not be loaded. Check the connection and reload the page.");
    }
    waiting.clear();
    worker = null;
  };
  return started;
}

// Failures are plain strings, as they are from the desktop app's backend.
function ask(request: Omit<EngineRequest, "id">): Promise<EngineReply> {
  worker ??= start();
  const id = ++lastId;
  return new Promise((resolve, reject) => {
    waiting.set(id, { resolve, reject });
    worker!.postMessage({ id, ...request } satisfies EngineRequest);
  });
}

/** Runs one of the engine's commands. `file` is the bytes of the file it works on, for those that take one. */
export async function engine<T>(command: string, args: unknown = {}, file?: Uint8Array): Promise<T> {
  const reply = await ask({ command, input: JSON.stringify(args ?? {}), file });
  return JSON.parse((reply as { result: string }).result) as T;
}

/** The picture being worked on, or its depth map, as a PNG file. Empty when there is none. */
export async function enginePicture(which: "image" | "depth"): Promise<Uint8Array> {
  const reply = await ask({ command: "picture", input: which });
  return (reply as { bytes: Uint8Array }).bytes;
}