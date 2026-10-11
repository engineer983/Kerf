import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export { isTauri };

export interface AppInfo {
  name: string;
  version: string;
}

export type Threshold = { mode: "otsu" } | { mode: "manual"; level: number };

export interface Preprocess {
  brightness: number;
  contrast: number;
  blurSigma: number;
  threshold: Threshold;
  invert: boolean;
}

export interface TraceSettings {
  mode: "outline" | "color";
  preprocess: Preprocess;
  speckle: number;
  curves: boolean;
  cornerThreshold: number;
  colorPrecision: number;
  layerDifference: number;
}

export interface Traced {
  width: number;
  height: number;
  pathCount: number;
  /** `<path>` elements in the traced image's pixel space. */
  body: string;
}

export type MachineKind = "laser" | "cnc";

/** Which bed corner is X0 Y0; from the back-left, Y increases towards the front. */
export type Origin = "frontLeft" | "backLeft";

export interface Machine {
  kind: MachineKind;
  origin: Origin;
  startGcode: string;
  endGcode: string;
  bedWidth: number;
  bedHeight: number;
  /** The S value that means full power (GRBL `$30`). */
  maxPowerS: number;
  travelFeed: number;
}

export type OperationKind =
  | { type: "laserCut" }
  | { type: "laserFill"; interval: number }
  /** Engraves the picture itself; `dither` turns shades into full-power dots. */
  | { type: "laserImage"; interval: number; dither: boolean }
  | {
      type: "cncProfile";
      depth: number;
      depthPerPass: number;
      plungeFeed: number;
      safeZ: number;
      spindleRpm: number;
    }
  | {
      /** Carves the image as a relief; brightness sets the surface height. */
      type: "cncRelief";
      depth: number;
      depthPerPass: number;
      plungeFeed: number;
      safeZ: number;
      spindleRpm: number;
      toolDiameter: number;
      ballNose: boolean;
      stepover: number;
      lightIsHigh: boolean;
      /** Side angle of a tapered ball nose in degrees; 0 for a straight bit. */
      taperAngle?: number;
      shankDiameter?: number;
    }
  | {
      /** Cuts a line around the artwork to full depth, to free the piece. */
      type: "cncCutout";
      depth: number;
      depthPerPass: number;
      plungeFeed: number;
      safeZ: number;
      spindleRpm: number;
      toolDiameter: number;
      /** How far outside the artwork the line is; negative is inside. */
      offset: number;
      /** Which side of the line the bit runs on. */
      side: "on" | "inside" | "outside";
      outline: "rectangle" | "ellipse";
      cornerRadius: number;
      tabs: number;
      tabWidth: number;
      tabHeight: number;
    }
  | {
      /**
       * The same line around the artwork, cut through by a laser. It runs in
       * the same job as the engraving, after it.
       */
      type: "laserCutout";
      offset: number;
      outline: "rectangle" | "ellipse";
      cornerRadius: number;
    };

export interface Operation {
  /** `#rrggbb`, or null for every shape. */
  color: string | null;
  kind: OperationKind;
  feed: number;
  power: number;
  passes: number;
}

export interface Job {
  machine: Machine;
  /** Top-left of the artwork, in mm from the bed's back-left corner. */
  offset: [number, number];
  operations: Operation[];
}

export interface Generated {
  lineCount: number;
  bounds: { minX: number; minY: number; maxX: number; maxY: number };
  cutMm: number;
  travelMm: number;
  seconds: number;
  /** SVG path data in bed coordinates. */
  cutPath: string;
  travelPath: string;
  /** Every move in order: `x, y, z, power` each, in bed coordinates. Power is -1 for a rapid. */
  moves: number[];
  /** Index in `moves` of the first move of each operation. */
  operationStarts: number[];
  head: string;
}

export function generateGcode(
  settings: TraceSettings,
  size: { widthMm: number; heightMm: number },
  job: Job,
  /** Carve reliefs from the estimated depth map, with this much photo texture (0 to 1). */
  depth: { enabled: boolean; detail: number; floor: number },
  /** Text and drawings added beside the artwork, as an SVG the size of the bed, with what makes them. */
  added: { svg: string; operations: Operation[] } | null = null,
  /** The space everything takes up on the bed (x, y, width, height), which cutouts go round. */
  cutoutAround: [number, number, number, number] | null = null,
): Promise<Generated> {
  return invoke("generate_gcode", {
    settings,
    widthMm: size.widthMm,
    heightMm: size.heightMm,
    job,
    depth,
    added,
    cutoutAround,
  });
}

/** Resolves to the saved path, or null if the user cancelled. */
export function exportGcode(suggestedName: string): Promise<string | null> {
  return invoke("export_gcode", { suggestedName });
}

export interface PortInfo {
  name: string;
  description: string | null;
}

export type MachineState =
  | "idle"
  | "run"
  | "hold"
  | "jog"
  | "alarm"
  | "door"
  | "check"
  | "home"
  | "sleep"
  | "other";

export type JobState = "running" | "finished" | "stopped" | "failed";

export type MachineEvent =
  | {
      type: "status";
      state: MachineState;
      position: [number, number, number] | null;
      feed: number | null;
      spindle: number | null;
    }
  | { type: "welcome"; version: string }
  | { type: "setting"; number: number; value: string }
  | { type: "line"; text: string }
  | { type: "commandError"; line: string; code: number; description: string }
  | { type: "alarm"; code: number; description: string }
  | { type: "job"; state: JobState; acknowledged: number; total: number; message: string | null }
  | { type: "disconnected"; reason: string | null };

export type MachineAction =
  | { type: "line"; text: string }
  | { type: "jog"; x: number; y: number; z: number; feed: number }
  | { type: "cancelJog" }
  | { type: "home" }
  | { type: "unlock" }
  | { type: "setOrigin"; z: boolean }
  | { type: "frame"; minX: number; minY: number; maxX: number; maxY: number; feed: number }
  | { type: "hold" }
  | { type: "resume" }
  | { type: "reset" }
  | { type: "startJob" }
  | { type: "stopJob" };

export function listPorts(): Promise<PortInfo[]> {
  return invoke("list_ports");
}

export function connectMachine(port: string): Promise<void> {
  return invoke("connect_machine", { port });
}

export function disconnectMachine(): Promise<void> {
  return invoke("disconnect_machine");
}

export function machineAction(action: MachineAction): Promise<void> {
  return invoke("machine_action", { action });
}

export function onMachineEvent(handler: (event: MachineEvent) => void): Promise<() => void> {
  return listen<MachineEvent>("machine", (e) => handler(e.payload));
}

/** Where the assistant sends requests, and which model it asks for. */
export interface Endpoint {
  protocol: "anthropic" | "openAi";
  baseUrl: string;
  model: string;
}

export interface ModelInfo {
  id: string;
  name: string;
}

/** One reply from the model. */
export interface AssistantTurn {
  text: string;
  toolCalls: { id: string; name: string; input: unknown }[];
  /** The reply in the provider's own format, to append to the history as is. */
  message: unknown;
  stop: { type: "done" } | { type: "toolUse" } | { type: "truncated" } | { type: "refused"; explanation: string };
  model: string;
}

/** Sends the conversation to the model. The key is looked up on the backend by `keyId`. */
export function assistantSend(
  endpoint: Endpoint,
  keyId: string,
  system: string,
  messages: unknown[],
  tools: { name: string; description: string; schema: unknown }[],
): Promise<AssistantTurn> {
  return invoke("assistant_send", { endpoint, keyId, system, messages, tools });
}

export function assistantModels(endpoint: Endpoint, keyId: string): Promise<ModelInfo[]> {
  return invoke("assistant_models", { endpoint, keyId });
}

/** Saves an API key in the operating system's credential store. It cannot be read back. */
export function setApiKey(keyId: string, key: string): Promise<void> {
  return invoke("set_api_key", { keyId, key });
}

export function hasApiKey(keyId: string): Promise<boolean> {
  return invoke("has_api_key", { keyId });
}

export function deleteApiKey(keyId: string): Promise<void> {
  return invoke("delete_api_key", { keyId });
}

/** What the gallery shows for a saved project. */
export interface ProjectMeta {
  id: string;
  name: string;
  savedAt: string;
  /** A small picture of the result, as a data URL. */
  thumbnail: string;
}

/** Saves the picture being worked on, with the UI's record of its settings. */
export function saveProject(meta: ProjectMeta, settings: string): Promise<void> {
  return invoke("save_project", { meta, settings });
}

export function listProjects(): Promise<ProjectMeta[]> {
  return invoke("list_projects");
}

/** Makes a saved project's picture the one being worked on; resolves to its settings. */
export function openProject(id: string): Promise<string> {
  return invoke("open_project", { id });
}

/** One of a saved project's pictures, as PNG bytes. */
export function projectPicture(id: string, which: "image" | "depth"): Promise<ArrayBuffer> {
  return invoke("project_picture", { id, which });
}

export function deleteProject(id: string): Promise<void> {
  return invoke("delete_project", { id });
}

/** Downloads a web page and returns its visible text. */
export function fetchPageText(url: string): Promise<{ title: string; text: string }> {
  return invoke("fetch_page_text", { url });
}

/** Returns null when the UI runs in a plain browser (`pnpm dev`) with no Rust backend. */
export async function getAppInfo(): Promise<AppInfo | null> {
  if (!isTauri()) return null;
  return invoke<AppInfo>("app_info");
}

export function loadImage(bytes: Uint8Array): Promise<void> {
  return invoke("load_image", bytes);
}

export function clearImage(): Promise<void> {
  return invoke("clear_image");
}

export function traceImage(settings: TraceSettings, preview: boolean): Promise<Traced> {
  return invoke("trace_image", { settings, preview });
}

/** Resolves to the saved path, or null if the user cancelled. */
export function exportSvg(
  settings: TraceSettings,
  widthMm: number,
  heightMm: number,
  suggestedName: string,
): Promise<string | null> {
  return invoke("export_svg", { settings, widthMm, heightMm, suggestedName });
}

/** Keeps only a rectangle of the working image, in its pixels. */
export function cropImage(x: number, y: number, width: number, height: number): Promise<void> {
  return invoke("crop_image", { x, y, width, height });
}

/** Undoes any crop. */
export function resetCrop(): Promise<void> {
  return invoke("reset_crop");
}

/** Sets the depth picture reliefs are carved from; an empty array removes it. */
export function setDepthMap(png: Uint8Array): Promise<void> {
  return invoke("set_depth_map", png);
}

/** Where pictures are made: an OpenAI-style image service, or a Stable Diffusion web UI. */
export interface ImageEndpoint {
  protocol: "openAi" | "webUi";
  baseUrl: string;
  model: string;
}

export type PictureShape = "square" | "wide" | "tall";

/** Makes a picture from a description and returns the image file's bytes. */
export function generatePicture(endpoint: ImageEndpoint, keyId: string, prompt: string, shape: PictureShape): Promise<ArrayBuffer> {
  return invoke("generate_picture", { endpoint, keyId, prompt, shape });
}

/** A picture found on the web that may be adapted and used commercially. */
export interface FoundPicture {
  title: string;
  creator: string;
  licence: string;
  needsCredit: boolean;
  attribution: string;
  page: string;
  url: string;
  thumbnail: string;
  width: number;
  height: number;
}

/** The first few pictures a search matched, and how many matched in all. */
export function searchPictures(query: string, publicDomainOnly: boolean): Promise<{ pictures: FoundPicture[]; total: number }> {
  return invoke("search_pictures", { query, publicDomainOnly });
}

/** Downloads a found picture, or its thumbnail, as the file's bytes. */
export function fetchPicture(url: string): Promise<ArrayBuffer> {
  return invoke("fetch_picture", { url });
}

/** Parts laid out to be cut from one piece of material, as an SVG in millimetres. */
export interface Sheet {
  svg: string;
  widthMm: number;
  heightMm: number;
  /** The names of the parts on the sheet, in reading order. */
  parts: string[];
}

export interface BoxSpec {
  /** Outside size, in millimetres. */
  width: number;
  depth: number;
  height: number;
  thickness: number;
  /** Zero chooses a finger width to suit the material. */
  fingerWidth: number;
  lid: boolean;
  /** The width of the cut itself: a laser's kerf, or a bit's diameter. */
  cutWidth: number;
  /** Carry the cut into inside corners, for a round bit. */
  dogbones: boolean;
}

export interface LayerSpec {
  layers: number;
  widthMm: number;
  smallestPieceMm: number;
  /** Draw on each layer where the next one sits. */
  marks: boolean;
}

/** Makes `svg` the design being cut in place of the traced picture; null goes back to the picture. */
export function setDesign(svg: string | null): Promise<void> {
  return invoke("set_design", { svg });
}

export function makeBox(spec: BoxSpec, sheetWidth: number, sheetHeight: number): Promise<Sheet[]> {
  return invoke("make_box", { spec, sheetWidth, sheetHeight });
}

/** Slices the loaded picture into layers to stack. */
export function makeLayers(
  spec: LayerSpec,
  settings: TraceSettings,
  lightIsHigh: boolean,
  depth: { enabled: boolean; detail: number; floor: number },
  sheetWidth: number,
  sheetHeight: number,
): Promise<Sheet[]> {
  return invoke("make_layers", { spec, settings, lightIsHigh, depth, sheetWidth, sheetHeight });
}

/** Words to draw as the outlines of their letters. */
export interface TextSpec {
  text: string;
  /** The name of a font family installed on this computer. */
  font: string;
  /** The font's size in millimetres; capitals stand about seven tenths of it. */
  sizeMm: number;
  bold: boolean;
  italic: boolean;
  align: "left" | "center" | "right";
  /**
   * How far the words bend round a circle, in degrees: 0 is straight, above 0
   * arches over the top, below 0 curves under the bottom.
   */
  bend?: number;
  /**
   * Set the words round the main artwork instead: following a circle, an
   * oval or the artwork's own frame, centred over its top or under its
   * bottom. The artwork's size is filled in when the words are made.
   */
  around?: {
    shape: "circle" | "oval" | "frame";
    side: "top" | "bottom";
    /** Clear space between the artwork and the words, in millimetres. */
    gap: number;
    width?: number;
    height?: number;
  } | null;
}

/** Words drawn as outlines. */
export interface TextMade {
  sheet: Sheet;
  /**
   * For words set round the artwork: where the sheet's top-left corner sits,
   * in millimetres right of and below the artwork's centre.
   */
  offset: [number, number] | null;
}

/** Reads an SVG document as plain lines, cropped to what is drawn. */
export function readSvg(svg: string, name: string): Promise<Sheet> {
  return invoke("read_svg", { svg, name });
}

/** Asks for a drawing file (SVG, or PDF, AI and EPS where Inkscape is installed) and reads it. */
export function openDrawing(): Promise<{ name: string; sheet: Sheet } | null> {
  return invoke("open_drawing");
}

/** Whether Inkscape is installed, which decides the file types that can be opened. */
export function inkscapeFound(): Promise<boolean> {
  return invoke("inkscape_found");
}

/** The font families installed on this computer. */
export function listFonts(): Promise<string[]> {
  return invoke("list_fonts");
}

export function makeText(spec: TextSpec): Promise<TextMade> {
  return invoke("make_text", { spec });
}
