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
  head: string;
}

export function generateGcode(
  settings: TraceSettings,
  widthMm: number,
  job: Job,
): Promise<Generated> {
  return invoke("generate_gcode", { settings, widthMm, job });
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
  suggestedName: string,
): Promise<string | null> {
  return invoke("export_svg", { settings, widthMm, suggestedName });
}
