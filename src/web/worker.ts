/**
 * Runs Kerf's engine (the Rust code, built as WebAssembly) off the page's own
 * thread, so that tracing and generating a job do not freeze the interface.
 */
import init, { call, picture } from "./pkg/kerf_web.js";

export interface EngineRequest {
  id: number;
  /** A command of the engine's, or `picture` for one of the pictures as a PNG. */
  command: string;
  /** The command's arguments as JSON; for `picture`, which one. */
  input: string;
  file?: Uint8Array;
}

export type EngineReply = { id: number } & ({ result: string } | { bytes: Uint8Array } | { error: string });

const ready = init();
const NOTHING = new Uint8Array();
const scope = self as unknown as { postMessage(message: EngineReply, transfer?: Transferable[]): void };

self.onmessage = async (event: MessageEvent<EngineRequest>) => {
  const { id, command, input, file } = event.data;
  try {
    await ready;
    if (command === "picture") {
      const bytes = picture(input);
      scope.postMessage({ id, bytes }, [bytes.buffer]);
    } else {
      scope.postMessage({ id, result: call(command, input, file ?? NOTHING) });
    }
  } catch (e) {
    // The engine's own refusals arrive as errors with a message to show. A
    // failure it did not expect arrives as a WebAssembly trap.
    const error =
      e instanceof WebAssembly.RuntimeError
        ? "Something went wrong inside Kerf. Reload the page; if it happens again with the same picture, please report it."
        : e instanceof Error
          ? e.message
          : String(e);
    scope.postMessage({ id, error });
  }
};