/**
 * The browser version's stand-in for `@tauri-apps/api/core`.
 *
 * The interface asks its backend for things by name. In the desktop app those
 * requests go to the Rust program around the window. Here most go to the same
 * Rust code built as WebAssembly, and the rest (files, saved projects, fonts,
 * web requests) are done with what a browser provides.
 */
import * as assistant from "./assistant";
import { engine } from "./engine";
import * as files from "./files";
import * as fonts from "./fonts";
import * as projects from "./projects";

/** The interface asks this before using its backend; here there is one. */
export function isTauri(): boolean {
  return true;
}

const needsDesktop = (what: string) => async (): Promise<never> => {
  throw `${what} needs the desktop app; it is not available in a browser.`;
};

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const handlers: Record<string, (args: any) => Promise<unknown>> = {
  export_gcode: files.exportGcode,
  export_svg: files.exportSvg,
  open_drawing: files.openDrawing,
  // Inkscape is a program on the computer, which a page cannot run.
  inkscape_found: async () => false,

  save_project: projects.saveProject,
  list_projects: projects.listProjects,
  open_project: projects.openProject,
  project_picture: projects.projectPicture,
  delete_project: projects.deleteProject,

  list_fonts: fonts.listFonts,
  make_text: fonts.makeText,

  set_api_key: assistant.setApiKey,
  has_api_key: assistant.hasApiKey,
  delete_api_key: assistant.deleteApiKey,
  assistant_send: assistant.assistantSend,
  assistant_models: assistant.assistantModels,
  generate_picture: assistant.generatePicture,
  fetch_picture: assistant.fetchPicture,
  search_pictures: needsDesktop("Searching the web for pictures"),
  fetch_page_text: needsDesktop("Reading a web page"),

  list_ports: async () => [],
  connect_machine: needsDesktop("Controlling a machine"),
  disconnect_machine: async () => undefined,
  machine_action: needsDesktop("Controlling a machine"),
};

export async function invoke<T>(command: string, args?: unknown): Promise<T> {
  const handler = handlers[command];
  if (handler) return (await handler(args)) as T;
  // A file's bytes are the whole of the request, as they are in the desktop app.
  if (args instanceof Uint8Array) return engine<T>(command, {}, args);
  if (args instanceof ArrayBuffer) return engine<T>(command, {}, new Uint8Array(args));
  return engine<T>(command, args);
}