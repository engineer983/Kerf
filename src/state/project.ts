import { create } from "zustand";
import { isAcceptedImage, loadImageFile } from "../lib/image";
import { estimateDepth } from "../lib/depth";
import { measureTones, optimise, PURPOSE_LABELS, type Purpose } from "../lib/optimize";
import * as ipc from "../lib/ipc";
import { CUSTOM_PRESET, MACHINE_PRESETS } from "../lib/machines";
import { cutSettings } from "../lib/feeds";
import { DEFAULT_MATERIAL, findMaterial, type Material } from "../lib/materials";
import { makeThumbnail } from "../lib/thumbnail";
import { suggestTool, type Tool } from "../lib/tools";
import { useToolbox } from "./tools";
import { usePlayback } from "./playback";
import type {
  Generated,
  MachineKind,
  Operation,
  OperationKind,
  Origin,
  Traced,
  TraceSettings,
} from "../lib/ipc";

export type Step = "device" | "import" | "trace" | "operations" | "machine";

export interface SourceImage {
  name: string;
  url: string;
  width: number;
  height: number;
  bytes: number;
  /** Continuous-tone, like a photograph, rather than line art. */
  photo: boolean;
}

export const DEFAULT_TRACE_SETTINGS: TraceSettings = {
  mode: "outline",
  preprocess: {
    brightness: 0,
    contrast: 0,
    blurSigma: 0,
    threshold: { mode: "otsu" },
    invert: false,
  },
  speckle: 4,
  curves: true,
  cornerThreshold: 60,
  colorPrecision: 6,
  layerDifference: 16,
};

/** An operation plus a stable key for list rendering. */
export type OperationItem = Operation & {
  id: number;
  /** The toolbox bit used, for router operations. Not sent to the generator. */
  toolId?: string | null;
  /** False to leave the operation out of the generated G-code. */
  enabled?: boolean;
};

/** A cut right through the material, a little outside the artwork, with tabs. */
export function defaultCutout(thickness: number): OperationKind {
  return {
    type: "cncCutout",
    depth: thickness,
    depthPerPass: 1,
    plungeFeed: 200,
    safeZ: 5,
    spindleRpm: 16000,
    toolDiameter: 3.175,
    offset: 5,
    side: "outside",
    outline: "rectangle",
    cornerRadius: 0,
    tabs: 4,
    tabWidth: 6,
    tabHeight: 1.5,
  };
}

let nextOperationId = 1;

/** Simple mode sets everything from a few choices; advanced shows every setting. */
export type Mode = "simple" | "advanced";

/** What the person wants made, in simple mode. Which ones apply depends on the machine. */
export type Goal = "picture" | "shapes" | "outline" | "carving" | "lines";
export type Quality = "draft" | "standard" | "fine";

export interface SimpleSetup {
  /** Null until chosen: the app then picks one to suit the machine and the picture. */
  goal: Goal | null;
  quality: Quality;
  /** How deep a carving goes, in millimetres. */
  depth: number;
  /** On a router, also cut the finished piece free. */
  cutOut: boolean;
  /** When the carving and the cut-out need different bits: which is made now. */
  part: "main" | "cutOut";
}

/** The goals a machine offers, most likely first. */
export function goalsFor(kind: MachineKind): Goal[] {
  return kind === "laser" ? ["picture", "shapes", "outline"] : ["carving", "lines"];
}

/** The goal used when none has been chosen. */
export function defaultGoal(kind: MachineKind, photo: boolean): Goal {
  return kind === "cnc" ? "carving" : photo ? "picture" : "shapes";
}

const MODE_KEY = "kerf.mode";

function storedMode(): Mode {
  try {
    return localStorage.getItem(MODE_KEY) === "advanced" ? "advanced" : "simple";
  } catch {
    return "simple";
  }
}

export const DEFAULT_RELIEF: OperationKind = {
  type: "cncRelief",
  depth: 3,
  depthPerPass: 1,
  plungeFeed: 200,
  safeZ: 5,
  spindleRpm: 16000,
  toolDiameter: 3.175,
  ballNose: true,
  stepover: 0.4,
  lightIsHigh: true,
};

/** Sets an operation's bit, and the speeds that suit it in `material`. */
export function withTool(op: OperationItem, tool: Tool, material: Material, spindleRpm: number): OperationItem {
  const settings = cutSettings(tool, material, spindleRpm);
  if (op.kind.type === "cncRelief") {
    return {
      ...op,
      toolId: tool.id,
      feed: settings?.feed ?? op.feed,
      kind: {
        ...op.kind,
        spindleRpm,
        toolDiameter: tool.diameter,
        ballNose: tool.kind === "ball" || tool.kind === "taperedBall",
        taperAngle: tool.kind === "taperedBall" ? (tool.taperAngle ?? 0) : 0,
        shankDiameter: tool.shank,
        ...(settings && {
          depthPerPass: settings.depthPerPass,
          plungeFeed: settings.plungeFeed,
          stepover: settings.stepover,
        }),
      },
    };
  }
  if (op.kind.type === "cncCutout") {
    return {
      ...op,
      toolId: tool.id,
      feed: settings?.feed ?? op.feed,
      kind: {
        ...op.kind,
        spindleRpm,
        toolDiameter: tool.diameter,
        ...(settings && { depthPerPass: settings.depthPerPass, plungeFeed: settings.plungeFeed }),
      },
    };
  }
  if (op.kind.type === "cncProfile") {
    return {
      ...op,
      toolId: tool.id,
      feed: settings?.feed ?? op.feed,
      kind: {
        ...op.kind,
        spindleRpm,
        ...(settings && { depthPerPass: settings.depthPerPass, plungeFeed: settings.plungeFeed }),
      },
    };
  }
  return op;
}

// Conservative starting points; real values depend on the machine and material.
// A photograph gets the operation that reproduces shading: image engraving on
// a laser, a relief on a router. Line art gets a fill or a path-following cut.
export function defaultOperation(kind: MachineKind, photo = false): OperationItem {
  const id = nextOperationId++;
  if (kind === "laser") {
    return photo
      ? { id, color: null, kind: { type: "laserImage", interval: 0.15, dither: true }, feed: 3000, power: 60, passes: 1 }
      : { id, color: null, kind: { type: "laserFill", interval: 0.1 }, feed: 3000, power: 50, passes: 1 };
  }
  return photo
    ? { id, color: null, kind: DEFAULT_RELIEF, feed: 600, power: 0, passes: 1 }
    : {
        id,
        color: null,
        kind: {
          type: "cncProfile",
          depth: 1,
          depthPerPass: 0.5,
          plungeFeed: 200,
          safeZ: 5,
          spindleRpm: 16000,
        },
        feed: 600,
        power: 0,
        passes: 1,
      };
}

interface TraceState {
  settings: TraceSettings;
  result: Traced | null;
  /** True while the result on screen comes from the low-resolution preview image. */
  refining: boolean;
  busy: boolean;
  error: string | null;
}

interface DepthState {
  status: "none" | "working" | "ready" | "error";
  /** Progress while working, or what went wrong. */
  message: string | null;
  /** The depth picture, for showing to the user. */
  url: string | null;
}

const NO_DEPTH: DepthState = { status: "none", message: null, url: null };

interface GcodeState {
  /** Null whenever anything it was generated from has changed since. */
  program: Generated | null;
  busy: boolean;
  error: string | null;
  exportedTo: string | null;
}

interface ProjectState {
  step: Step;
  image: SourceImage | null;
  /** Work area in millimetres. */
  bed: { width: number; height: number };
  machineKind: MachineKind;
  maxPowerS: number;
  /** Id of the machine preset in use, or `CUSTOM_PRESET` once any value is edited. */
  machinePreset: string;
  origin: Origin;
  /** Vendor commands emitted around every job; set by presets. */
  startGcode: string;
  endGcode: string;
  /** Physical size of the artwork in millimetres. */
  widthMm: number;
  heightMm: number;
  /** Keep the artwork's proportions when its width or height changes. */
  aspectLocked: boolean;
  /** The image before any crop, so that a crop can be undone. */
  uncropped: SourceImage | null;
  /** True while the crop frame is shown on the canvas. */
  cropping: boolean;
  /** What a relief takes its heights from. */
  heightSource: "brightness" | "depth";
  depth: DepthState;
  /** How much of the photo's fine texture is laid over a depth relief, 0 to 1. */
  detail: number;
  mode: Mode;
  simple: SimpleSetup;
  setMode: (mode: Mode) => void;
  /** Changes a simple-mode choice and sets the project up to match. */
  setSimple: (patch: Partial<SimpleSetup>) => void;
  /** Share of the depth range, from the far end, flattened into a level background (0 to 0.9). */
  depthFloor: number;
  /** Distance of the artwork's left edge from the bed's left edge, in mm. */
  left: number;
  /**
   * Distance of the artwork from the bed edge on the machine's origin side:
   * from the front for a front-left origin, from the back for a back-left one.
   */
  front: number;
  /** Size of the piece of material, which sits in the bed's origin corner. */
  stockWidth: number;
  stockLength: number;
  /** Router speed in rpm, used to work out feeds. */
  spindleRpm: number;
  showSource: boolean;
  /** The material shown in previews; it does not change the G-code. */
  materialId: string;
  /** Material thickness in millimetres, for previews. */
  thickness: number;
  error: string | null;
  trace: TraceState;
  /** Path of the last exported SVG, for confirmation in the UI. */
  exportedTo: string | null;
  operations: OperationItem[];
  gcode: GcodeState;
  /**
   * Moves to a step. Going back from a generated job to the picture clears
   * the job, so that is asked about first unless the person has said not to.
   */
  setStep: (step: Step) => void;
  /** The earlier step waiting on the person's answer, while they are asked. */
  pendingStep: Step | null;
  confirmStep: (dontWarnAgain: boolean) => void;
  cancelStep: () => void;
  /** What the last optimisation did, for showing beside the button. */
  optimizeNote: string | null;
  /** Tunes the picture's adjustments to what is being made; returns what it did. */
  optimizePicture: () => Promise<string>;
  /**
   * Starts a project from a picture. `drawnHeightMap` says the picture was
   * drawn so that its brightness is its height, which a carving should then
   * follow directly instead of estimating depth.
   */
  importFile: (file: File, options?: { drawnHeightMap?: boolean }) => Promise<void>;
  clearImage: () => void;
  /** Saves the current project, then clears everything for a fresh start. */
  newProject: () => Promise<void>;
  /** True when the picture is a drawn height map: brightness is height. */
  drawnHeightMap: boolean;
  setWidthMm: (widthMm: number) => void;
  setHeightMm: (heightMm: number) => void;
  setAspectLocked: (locked: boolean) => void;
  /** Moves and sizes the artwork, given as a rectangle in bed coordinates. */
  setArtworkRect: (rect: { x: number; y: number; width: number; height: number }) => void;
  setCropping: (cropping: boolean) => void;
  /** Keeps the part of the image inside the fractions given (0 to 1 of its width and height). */
  applyCrop: (crop: { x0: number; y0: number; x1: number; y1: number }) => Promise<void>;
  resetCrop: () => Promise<void>;
  setHeightSource: (source: "brightness" | "depth") => void;
  setDetail: (detail: number) => void;
  setDepthFloor: (depthFloor: number) => void;
  /** Runs the depth model on the image. Downloads the model the first time. */
  estimateDepth: () => Promise<void>;
  setPosition: (patch: { left?: number; front?: number }) => void;
  setBed: (patch: { width?: number; height?: number }) => void;
  setMachineKind: (kind: MachineKind) => void;
  setMaxPowerS: (maxPowerS: number) => void;
  setOrigin: (origin: Origin) => void;
  applyMachinePreset: (id: string) => void;
  setShowSource: (show: boolean) => void;
  /** Identifies this piece of work among the saved projects. */
  projectId: string | null;
  /** Saved projects, newest first. */
  projects: ipc.ProjectMeta[];
  /** Why the last save or open failed, if it did. */
  galleryError: string | null;
  refreshProjects: () => Promise<void>;
  /** Saves the picture and its settings so the project can be opened again. */
  saveProject: () => Promise<void>;
  openProject: (id: string) => Promise<void>;
  deleteProject: (id: string) => Promise<void>;
  /** Whether there is a change to step back from, or forward to. */
  canUndo: boolean;
  canRedo: boolean;
  undo: () => void;
  redo: () => void;
  /** Puts back a set of settings saved by the undo history. */
  restore: (snapshot: Snapshot) => void;
  setMaterial: (id: string) => void;
  setThickness: (thickness: number) => void;
  setStock: (patch: { stockWidth?: number; stockLength?: number }) => void;
  setSpindleRpm: (spindleRpm: number) => void;
  /** Changes what an operation does, giving it a suitable bit and speeds. */
  setOperationType: (id: number, type: OperationKind["type"]) => void;
  /** Changes an operation's bit and recalculates its speeds. */
  setOperationTool: (id: number, toolId: string) => void;
  updateTrace: (patch: Partial<TraceSettings>) => void;
  updatePreprocess: (patch: Partial<TraceSettings["preprocess"]>) => void;
  exportSvg: () => Promise<void>;
  addOperation: () => void;
  updateOperation: (id: number, patch: Partial<OperationItem>) => void;
  /** Adds an operation that cuts the piece free, after the others. */
  addCutout: () => void;
  removeOperation: (id: number) => void;
  generateGcode: () => Promise<void>;
  exportGcode: () => Promise<void>;
}

const EMPTY_TRACE: TraceState = {
  settings: DEFAULT_TRACE_SETTINGS,
  result: null,
  refining: false,
  busy: false,
  error: null,
};

const EMPTY_GCODE: GcodeState = { program: null, busy: false, error: null, exportedTo: null };

const PREVIEW_DELAY_MS = 120;
const REFINE_DELAY_MS = 500;
const TRAVEL_FEED = 6000;

let traceTimer: ReturnType<typeof setTimeout> | undefined;
// Bumped on every request so that a slow, outdated result cannot overwrite a newer one.
let traceGeneration = 0;
let gcodeGeneration = 0;

const tenth = (mm: number) => Math.round(mm * 10) / 10;

/** The longest side given to newly imported artwork, in mm, on a large bed. */
const DEFAULT_LONG_SIDE_MM = 200;

function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** Where the artwork sits on the bed, in bed coordinates (mm, Y down from the back). */
export function artworkRect(
  s: Pick<ProjectState, "image" | "widthMm" | "heightMm" | "left" | "front" | "bed" | "origin">,
) {
  if (!s.image) return null;
  const height = s.heightMm;
  const y = s.origin === "backLeft" ? s.front : s.bed.height - s.front - height;
  return { x: s.left, y, width: s.widthMm, height };
}

/** Where the piece of material sits: in the bed's origin corner. */
export function stockRect(s: Pick<ProjectState, "stockWidth" | "stockLength" | "bed" | "origin">) {
  const y = s.origin === "backLeft" ? 0 : s.bed.height - s.stockLength;
  return { x: 0, y, width: s.stockWidth, height: s.stockLength };
}

/** Gap left around the artwork when the material size is set automatically, in mm. */
const STOCK_MARGIN_MM = 10;

/** Distinct fill colours in a trace, lower-cased, in first-seen order. */
export function traceColors(traced: Traced | null): string[] {
  if (!traced) return [];
  const colors = new Set<string>();
  for (const match of traced.body.matchAll(/fill="(#[0-9a-fA-F]{6})"/g)) {
    colors.add(match[1].toLowerCase());
  }
  return [...colors];
}

/** The settings that undo and redo step through. */
const UNDOABLE = [
  "widthMm",
  "heightMm",
  "aspectLocked",
  "left",
  "front",
  "stockWidth",
  "stockLength",
  "materialId",
  "thickness",
  "operations",
  "heightSource",
  "detail",
  "depthFloor",
] as const;

type Snapshot = Pick<ProjectState, (typeof UNDOABLE)[number]> & { traceSettings: TraceSettings };

function snapshot(s: ProjectState): Snapshot {
  const picked = Object.fromEntries(UNDOABLE.map((key) => [key, s[key]])) as Pick<
    ProjectState,
    (typeof UNDOABLE)[number]
  >;
  return { ...picked, traceSettings: s.trace.settings };
}

const sameSnapshot = (a: Snapshot, b: Snapshot) =>
  a.traceSettings === b.traceSettings && UNDOABLE.every((key) => a[key] === b[key]);

const HISTORY_LIMIT = 100;
/** Changes closer together than this, such as a slider being dragged, undo as one step. */
const SAME_STEP_MS = 600;

const past: Snapshot[] = [];
const future: Snapshot[] = [];
let stepping = false;
let lastChange = 0;

/** Moves one step along the history: from `from`, leaving the current state on `to`. */
function stepHistory(from: Snapshot[], to: Snapshot[]) {
  const target = from.pop();
  if (!target) return;
  to.push(snapshot(useProject.getState()));
  stepping = true;
  useProject.getState().restore(target);
  stepping = false;
  useProject.setState({ canUndo: past.length > 0, canRedo: future.length > 0 });
}

/** The order the steps are worked through. */
export const STEP_ORDER: Step[] = ["device", "import", "trace", "operations", "machine"];
const WARN_ON_BACK_KEY = "kerf.warnOnBack";

function warnsOnBack(): boolean {
  try {
    return localStorage.getItem(WARN_ON_BACK_KEY) !== "off";
  } catch {
    return true;
  }
}

const DEVICE_KEY = "kerf.device";
const DEVICE_KEYS = [
  "machinePreset",
  "machineKind",
  "bed",
  "origin",
  "maxPowerS",
  "spindleRpm",
  "startGcode",
  "endGcode",
] as const;

/** The machine settings that are remembered between sessions. */
type Device = Pick<
  ProjectState,
  "machinePreset" | "machineKind" | "bed" | "origin" | "maxPowerS" | "spindleRpm" | "startGcode" | "endGcode"
>;

function storedDevice(): Device | null {
  try {
    const device = JSON.parse(localStorage.getItem(DEVICE_KEY) ?? "null");
    return device && typeof device.machineKind === "string" && device.bed ? (device as Device) : null;
  } catch {
    return null;
  }
}

export const useProject = create<ProjectState>((set, get) => {
  async function runTrace(preview: boolean) {
    const generation = ++traceGeneration;
    const patchTrace = (patch: Partial<TraceState>) =>
      set((s) => ({ trace: { ...s.trace, ...patch } }));

    patchTrace({ busy: true });
    try {
      const result = await ipc.traceImage(get().trace.settings, preview);
      if (generation !== traceGeneration) return;
      patchTrace({ result, refining: preview, busy: preview, error: null });
      if (preview) {
        traceTimer = setTimeout(() => void runTrace(false), REFINE_DELAY_MS);
      }
    } catch (e) {
      if (generation !== traceGeneration) return;
      patchTrace({ busy: false, refining: false, error: message(e) });
    }
  }

  function scheduleTrace() {
    if (!get().image || !ipc.isTauri()) return;
    clearTimeout(traceTimer);
    traceGeneration++;
    traceTimer = setTimeout(() => void runTrace(true), PREVIEW_DELAY_MS);
  }

  function cancelTrace() {
    clearTimeout(traceTimer);
    traceGeneration++;
  }

  /** Gives a router operation the toolbox bit that suits it, with speeds for the material. */
  function equip(op: OperationItem, s: ProjectState = get()): OperationItem {
    if (op.kind.type !== "cncRelief" && op.kind.type !== "cncProfile" && op.kind.type !== "cncCutout") {
      return op;
    }
    const { tools } = useToolbox.getState();
    const rect = artworkRect(s);
    const shortSide = rect ? Math.min(rect.width, rect.height) : 150;
    const job = op.kind.type === "cncRelief" ? "relief" : "profile";
    const tool = tools.find((t) => t.id === op.toolId) ?? suggestTool(job, tools, shortSide);
    return tool ? withTool(op, tool, findMaterial(s.materialId), s.spindleRpm) : op;
  }

  /** A starting operation for the machine and the kind of image loaded. */
  function fresh(kind: MachineKind, s: ProjectState = get()): OperationItem {
    return equip(defaultOperation(kind, s.image?.photo ?? false), s);
  }

  /**
   * Sets the whole project up from the simple-mode choices: which operations
   * to run, with which bit, and at what settings.
   */
  async function applySimple() {
    const s = get();
    if (!s.image) return;
    const { quality, cutOut } = s.simple;
    const goal =
      s.simple.goal && goalsFor(s.machineKind).includes(s.simple.goal)
        ? s.simple.goal
        : defaultGoal(s.machineKind, s.image.photo);
    const material = findMaterial(s.materialId);
    const blank = { color: null, power: 0, passes: 1, feed: 600 };
    const operations: OperationItem[] = [];

    if (s.machineKind === "laser") {
      // Finer lines hold more detail and take longer. About 0.1 mm is as
      // fine as a diode laser's spot allows.
      const interval = { draft: 0.2, standard: 0.15, fine: 0.1 }[quality];
      // Speed and power depend on the laser and the material, which the app
      // does not know, so whatever is already set is kept.
      const current = s.operations.find((op) => op.kind.type.startsWith("laser"));
      const kind: OperationKind =
        goal === "picture"
          ? { type: "laserImage", interval, dither: true }
          : goal === "shapes"
            ? { type: "laserFill", interval }
            : { type: "laserCut" };
      operations.push({
        ...blank,
        id: nextOperationId++,
        kind,
        feed: current?.feed ?? 3000,
        power: current?.power ?? 50,
        passes: current?.passes ?? 1,
      });
    } else {
      const { tools } = useToolbox.getState();
      const rect = artworkRect(s);
      const shortSide = rect ? Math.min(rect.width, rect.height) : 150;
      if (goal === "carving") {
        // The bit's tip sets how much detail survives: the finest tip for
        // fine work, the widest for a quick draft.
        const rounded = tools.filter((t) => t.kind === "ball" || t.kind === "taperedBall");
        const byTip = [...rounded].sort((a, b) => a.diameter - b.diameter);
        const tool =
          quality === "fine" ? byTip[0] : quality === "draft" ? byTip.at(-1) : suggestTool("relief", tools, shortSide);
        // Leave some material under the deepest cut.
        const depth = Math.max(0.5, Math.min(s.simple.depth, s.thickness - 2));
        const carving: OperationItem = { ...blank, id: nextOperationId++, kind: { ...DEFAULT_RELIEF, depth } as OperationKind };
        operations.push(tool ? withTool(carving, tool, material, s.spindleRpm) : carving);
      } else {
        operations.push(equip({ ...defaultOperation("cnc"), id: nextOperationId++ }, s));
      }
      if (cutOut) {
        operations.push(equip({ ...blank, id: nextOperationId++, kind: defaultCutout(s.thickness) }, s));
      }
      // Two bits cannot be used in one run, so only one part is made at a time.
      const [main, cut] = operations;
      if (cut && main.toolId !== cut.toolId) {
        main.enabled = s.simple.part === "main";
        cut.enabled = s.simple.part === "cutOut";
      }
    }

    set({
      operations,
      // A light touch: enough to bring out linework such as feathers and
      // lettering without roughening the form. A photograph gets less still,
      // or skin turns rough.
      detail: s.image.photo ? 0.1 : 0.15,
      depthFloor: 0.3,
      ...staleGcode(),
    });

    // A carving is shaped from estimated depth, which is worked out once per
    // picture. A drawn height map already says how high each part is.
    if (goal === "carving" && get().drawnHeightMap) {
      if (get().heightSource !== "brightness") get().setHeightSource("brightness");
    } else if (goal === "carving") {
      if (get().depth.status === "none" || get().depth.status === "error") await get().estimateDepth();
      if (get().depth.status === "ready" && get().heightSource !== "depth") get().setHeightSource("depth");
    }
  }

  /** Drops any generated G-code; call when one of its inputs changes. */
  function staleGcode(): { gcode: GcodeState } {
    gcodeGeneration++;
    return { gcode: EMPTY_GCODE };
  }

  // A machine chosen in an earlier session is offered again. The app always
  // opens on the first step, so the work runs in order from there.
  const remembered = storedDevice();

  /** What the picture is being used for, judged from the job as it stands. */
  function purpose(): Purpose {
    const s = get();
    const included = s.operations.filter((op) => op.enabled !== false);
    if (included.some((op) => op.kind.type === "cncRelief")) return "carving";
    if (included.some((op) => op.kind.type === "laserImage")) return "engraving";
    return "shapes";
  }

  return {
    step: "device",
    image: null,
    bed: { width: 400, height: 400 },
    machineKind: "laser",
    maxPowerS: 1000,
    machinePreset: CUSTOM_PRESET,
    origin: "frontLeft",
    startGcode: "",
    endGcode: "",
    spindleRpm: 16000,
    ...remembered,
    widthMm: 100,
    heightMm: 100,
    aspectLocked: true,
    uncropped: null,
    cropping: false,
    heightSource: "brightness",
    depth: NO_DEPTH,
    detail: 0.15,
    depthFloor: 0,
    mode: storedMode(),
    simple: { goal: null, quality: "standard", depth: 4, cutOut: false, part: "main" },

    setMode: (mode) => {
      try {
        localStorage.setItem(MODE_KEY, mode);
      } catch {
        // Not persisted; the choice still applies for this session.
      }
      set({ mode });
      // Going back to simple puts the project back under its automatic setup.
      if (mode === "simple") void applySimple();
    },
    setSimple: (patch) => {
      set((s) => ({ simple: { ...s.simple, ...patch } }));
      void applySimple();
    },
    left: STOCK_MARGIN_MM,
    front: STOCK_MARGIN_MM,
    stockWidth: 300,
    stockLength: 200,
    showSource: true,
    materialId: DEFAULT_MATERIAL.id,
    thickness: DEFAULT_MATERIAL.thickness,
    error: null,
    trace: EMPTY_TRACE,
    exportedTo: null,
    operations: [defaultOperation(remembered?.machineKind ?? "laser")],
    gcode: EMPTY_GCODE,

    setStep: (step) => {
      const s = get();
      const made = STEP_ORDER.indexOf("operations");
      const goingBack = STEP_ORDER.indexOf(s.step) >= made && STEP_ORDER.indexOf(step) < made;
      if (!goingBack || !s.gcode.program) {
        set({ step });
      } else if (warnsOnBack()) {
        set({ pendingStep: step });
      } else {
        // The canvas goes back to showing the picture, not the old job.
        set({ step, ...staleGcode() });
      }
    },
    pendingStep: null,
    confirmStep: (dontWarnAgain) => {
      const step = get().pendingStep;
      if (!step) return;
      if (dontWarnAgain) {
        try {
          localStorage.setItem(WARN_ON_BACK_KEY, "off");
        } catch {
          // Not remembered; they will be asked again next time.
        }
      }
      set({ step, pendingStep: null, ...staleGcode() });
    },
    cancelStep: () => set({ pendingStep: null }),

    optimizeNote: null,
    optimizePicture: async () => {
      const s = get();
      if (!s.image) throw new Error("There is no picture loaded yet.");
      const result = optimise(purpose(), await measureTones(s.image.url), s.trace.settings);
      const note = result.changes.length
        ? `Tuned for ${PURPOSE_LABELS[result.purpose]}: ${result.changes.join(", ")}.`
        : `Already tuned for ${PURPOSE_LABELS[result.purpose]}; nothing needed changing.`;
      if (result.changes.length) get().updateTrace(result.settings);
      set({ optimizeNote: note });
      return note;
    },

    drawnHeightMap: false,

    newProject: async () => {
      // What was being worked on is kept, so starting again loses nothing.
      if (get().image) await get().saveProject();
      get().clearImage();
      set((s) => ({
        projectId: null,
        step: "device",
        pendingStep: null,
        optimizeNote: null,
        trace: EMPTY_TRACE,
        simple: { goal: null, quality: "standard", depth: 4, cutOut: false, part: "main" },
        operations: [fresh(s.machineKind)],
        heightSource: "brightness",
        detail: 0.15,
        depthFloor: 0,
        drawnHeightMap: false,
        aspectLocked: true,
        ...staleGcode(),
      }));
    },

    importFile: async (file, options) => {
      if (!isAcceptedImage(file)) {
        set({ error: `${file.name} is not a supported image (PNG, JPEG, WebP, BMP or GIF).` });
        return;
      }
      try {
        const image = await loadImageFile(file);
        if (ipc.isTauri()) {
          try {
            await ipc.loadImage(new Uint8Array(await file.arrayBuffer()));
          } catch (e) {
            URL.revokeObjectURL(image.url);
            throw e;
          }
        }
        cancelTrace();
        const previous = get().image;
        if (previous) URL.revokeObjectURL(previous.url);

        // Start at half the bed, whichever side is the tighter fit, on a piece
        // of material just larger than the artwork in the origin corner.
        const { bed, trace, machineKind } = get();
        const scale = Math.min(
          (bed.width * 0.5) / image.width,
          (bed.height * 0.5) / image.height,
          DEFAULT_LONG_SIDE_MM / Math.max(image.width, image.height),
        );
        const widthMm = Math.round(image.width * scale);
        const heightMm = (widthMm * image.height) / image.width;
        const placed = {
          image,
          widthMm,
          heightMm: Math.round(heightMm * 10) / 10,
          aspectLocked: true,
          uncropped: null,
          cropping: false,
          drawnHeightMap: options?.drawnHeightMap ?? false,
          heightSource: "brightness" as const,
          depth: NO_DEPTH,
          left: STOCK_MARGIN_MM,
          front: STOCK_MARGIN_MM,
          stockWidth: widthMm + STOCK_MARGIN_MM * 2,
          stockLength: Math.ceil(heightMm) + STOCK_MARGIN_MM * 2,
        };
        set({
          ...placed,
          projectId: `p-${Date.now().toString(36)}`,
          operations: [fresh(machineKind, { ...get(), ...placed })],
          error: null,
          exportedTo: null,
          optimizeNote: null,
          trace: { ...EMPTY_TRACE, settings: trace.settings },
          // Simple mode needs no tracing decisions, so it goes straight to the job.
          step: get().mode === "simple" ? "operations" : "trace",
          ...staleGcode(),
        });
        scheduleTrace();
        if (get().mode === "simple") void applySimple();
      } catch (e) {
        set({ error: message(e) });
      }
    },

    clearImage: () => {
      cancelTrace();
      const previous = get().image;
      if (previous) URL.revokeObjectURL(previous.url);
      if (ipc.isTauri()) void ipc.clearImage();
      set((s) => ({
        image: null,
        error: null,
        exportedTo: null,
        trace: { ...EMPTY_TRACE, settings: s.trace.settings },
        ...staleGcode(),
      }));
    },

    // With the proportions locked, changing one side scales the other with it.
    setWidthMm: (widthMm) =>
      set((s) => ({
        widthMm,
        heightMm: s.aspectLocked ? Math.round(((s.heightMm * widthMm) / s.widthMm) * 10) / 10 : s.heightMm,
        ...staleGcode(),
      })),
    setHeightMm: (heightMm) =>
      set((s) => ({
        heightMm,
        widthMm: s.aspectLocked ? Math.round(((s.widthMm * heightMm) / s.heightMm) * 10) / 10 : s.widthMm,
        ...staleGcode(),
      })),
    setAspectLocked: (aspectLocked) => set({ aspectLocked }),
    setArtworkRect: (rect) =>
      set((s) => ({
        // A tenth of a millimetre is finer than either machine can place work.
        widthMm: tenth(rect.width),
        heightMm: tenth(rect.height),
        left: tenth(rect.x),
        front: tenth(s.origin === "backLeft" ? rect.y : s.bed.height - rect.y - rect.height),
        ...staleGcode(),
      })),
    setCropping: (cropping) => set({ cropping }),

    applyCrop: async ({ x0, y0, x1, y1 }) => {
      const s = get();
      const { image } = s;
      const rect = artworkRect(s);
      if (!image || !rect) return;
      const x = Math.round(x0 * image.width);
      const y = Math.round(y0 * image.height);
      const width = Math.max(1, Math.round((x1 - x0) * image.width));
      const height = Math.max(1, Math.round((y1 - y0) * image.height));
      try {
        // The same crop is made here, for display, and in the backend, for tracing.
        const picture = new Image();
        picture.src = image.url;
        await picture.decode();
        const canvas = document.createElement("canvas");
        canvas.width = width;
        canvas.height = height;
        canvas.getContext("2d")!.drawImage(picture, -x, -y);
        const blob = await new Promise<Blob | null>((done) => canvas.toBlob(done, "image/png"));
        if (!blob) throw new Error("The cropped image could not be made.");
        if (ipc.isTauri()) await ipc.cropImage(x, y, width, height);

        cancelTrace();
        if (s.depth.url) URL.revokeObjectURL(s.depth.url);
        if (s.uncropped) URL.revokeObjectURL(image.url);
        set({
          image: { ...image, url: URL.createObjectURL(blob), width, height, bytes: blob.size },
          uncropped: s.uncropped ?? image,
          cropping: false,
          depth: NO_DEPTH,
          heightSource: "brightness",
          trace: { ...EMPTY_TRACE, settings: s.trace.settings },
          error: null,
        });
        // The part that is kept stays where it was on the material.
        get().setArtworkRect({
          x: rect.x + x0 * rect.width,
          y: rect.y + y0 * rect.height,
          width: (x1 - x0) * rect.width,
          height: (y1 - y0) * rect.height,
        });
        scheduleTrace();
      } catch (e) {
        set({ error: message(e), cropping: false });
      }
    },

    resetCrop: async () => {
      const s = get();
      if (!s.uncropped || !s.image) return;
      try {
        if (ipc.isTauri()) await ipc.resetCrop();
        cancelTrace();
        URL.revokeObjectURL(s.image.url);
        if (s.depth.url) URL.revokeObjectURL(s.depth.url);
        set({
          image: s.uncropped,
          uncropped: null,
          cropping: false,
          depth: NO_DEPTH,
          heightSource: "brightness",
          // Keep the width; the height returns to the whole image's proportions.
          heightMm: Math.round(((s.widthMm * s.uncropped.height) / s.uncropped.width) * 10) / 10,
          trace: { ...EMPTY_TRACE, settings: s.trace.settings },
          ...staleGcode(),
        });
        scheduleTrace();
      } catch (e) {
        set({ error: message(e) });
      }
    },

    setHeightSource: (heightSource) => set({ heightSource, ...staleGcode() }),
    setDetail: (detail) => set({ detail, ...staleGcode() }),
    setDepthFloor: (depthFloor) => set({ depthFloor, ...staleGcode() }),

    estimateDepth: async () => {
      const { image } = get();
      if (!image || get().depth.status === "working") return;
      const working = (text: string) =>
        set({ depth: { status: "working", message: text, url: null } });
      working("Starting…");
      try {
        const png = await estimateDepth(image.url, working);
        // The image may have been replaced or cropped while the model ran.
        if (get().image !== image) return;
        await ipc.setDepthMap(new Uint8Array(await png.arrayBuffer()));
        set({
          depth: { status: "ready", message: null, url: URL.createObjectURL(png) },
          heightSource: "depth",
          ...staleGcode(),
        });
      } catch (e) {
        if (get().image !== image) return;
        set({ depth: { status: "error", message: message(e), url: null } });
      }
    },
    setPosition: (patch) => set({ ...patch, ...staleGcode() }),
    setBed: (patch) =>
      set((s) => ({
        bed: { ...s.bed, ...patch },
        machinePreset: CUSTOM_PRESET,
        ...staleGcode(),
      })),
    setMachineKind: (machineKind) =>
      set({
        machineKind,
        machinePreset: CUSTOM_PRESET,
        operations: [fresh(machineKind)],
        ...staleGcode(),
      }),
    setMaxPowerS: (maxPowerS) => set({ maxPowerS, machinePreset: CUSTOM_PRESET, ...staleGcode() }),
    setOrigin: (origin) => set({ origin, machinePreset: CUSTOM_PRESET, ...staleGcode() }),
    applyMachinePreset: (id) => {
      const preset = MACHINE_PRESETS.find((p) => p.id === id);
      if (!preset) {
        set({ machinePreset: CUSTOM_PRESET });
        return;
      }
      set((s) => ({
        machinePreset: preset.id,
        bed: preset.bed,
        maxPowerS: preset.maxPowerS,
        origin: preset.origin,
        startGcode: preset.startGcode,
        endGcode: preset.endGcode,
        machineKind: preset.kind,
        // Operations are specific to the machine type; keep them when it is unchanged.
        operations: s.machineKind === preset.kind ? s.operations : [fresh(preset.kind, s)],
        ...staleGcode(),
      }));
    },
    setShowSource: (showSource) => set({ showSource }),
    projectId: null,
    projects: [],
    galleryError: null,

    refreshProjects: async () => {
      try {
        set({ projects: await ipc.listProjects() });
      } catch (e) {
        set({ galleryError: message(e) });
      }
    },

    saveProject: async () => {
      const s = get();
      if (!s.image || !s.projectId || !ipc.isTauri()) return;
      try {
        const { url: _url, ...image } = s.image;
        const device = Object.fromEntries(DEVICE_KEYS.map((key) => [key, s[key]]));
        await ipc.saveProject(
          {
            id: s.projectId,
            name: s.image.name.replace(/\.[^.]+$/, ""),
            savedAt: new Date().toISOString(),
            thumbnail: await makeThumbnail(s.image.url),
          },
          JSON.stringify({ image, device, settings: snapshot(s), hasDepth: s.depth.status === "ready" }),
        );
        set({ galleryError: null });
        await get().refreshProjects();
      } catch (e) {
        set({ galleryError: `The project could not be saved: ${message(e)}` });
      }
    },

    openProject: async (id) => {
      try {
        const saved = JSON.parse(await ipc.openProject(id)) as {
          image: Omit<SourceImage, "url">;
          device: Device;
          settings: Snapshot;
          hasDepth: boolean;
        };
        const picture = async (which: "image" | "depth") =>
          URL.createObjectURL(new Blob([await ipc.projectPicture(id, which)], { type: "image/png" }));
        const url = await picture("image");
        const depthUrl = saved.hasDepth ? await picture("depth") : null;

        cancelTrace();
        const previous = get();
        if (previous.image) URL.revokeObjectURL(previous.image.url);
        if (previous.uncropped) URL.revokeObjectURL(previous.uncropped.url);
        if (previous.depth.url) URL.revokeObjectURL(previous.depth.url);
        // New operations must not reuse the ids of the ones being loaded.
        nextOperationId = Math.max(nextOperationId, ...saved.settings.operations.map((op) => op.id + 1));
        set({
          ...saved.device,
          image: { ...saved.image, url },
          uncropped: null,
          cropping: false,
          depth: depthUrl ? { status: "ready", message: null, url: depthUrl } : NO_DEPTH,
          projectId: id,
          // A saved project keeps exactly the settings it was saved with, so
          // it opens with every setting showing rather than being set up afresh.
          mode: "advanced",
          step: "operations",
          error: null,
          exportedTo: null,
          galleryError: null,
          trace: { ...EMPTY_TRACE, settings: saved.settings.traceSettings },
        });
        get().restore(saved.settings);
        scheduleTrace();
      } catch (e) {
        set({ galleryError: `The project could not be opened: ${message(e)}` });
      }
    },

    deleteProject: async (id) => {
      try {
        await ipc.deleteProject(id);
        await get().refreshProjects();
      } catch (e) {
        set({ galleryError: message(e) });
      }
    },

    canUndo: false,
    canRedo: false,
    undo: () => stepHistory(past, future),
    redo: () => stepHistory(future, past),
    restore: ({ traceSettings, ...rest }) => {
      const retrace = traceSettings !== get().trace.settings;
      set((s) => ({ ...rest, trace: { ...s.trace, settings: traceSettings }, ...staleGcode() }));
      if (retrace) scheduleTrace();
    },
    setMaterial: (materialId) =>
      set((s) => {
        const next = { ...s, materialId };
        // Speeds depend on the material, so router operations are recalculated.
        return {
          materialId,
          thickness: findMaterial(materialId).thickness,
          operations: s.operations.map((op) => equip(op, next)),
          ...staleGcode(),
        };
      }),
    setThickness: (thickness) => set({ thickness }),
    setStock: (patch) => set(patch),
    setSpindleRpm: (spindleRpm) =>
      set((s) => ({
        spindleRpm,
        operations: s.operations.map((op) => equip(op, { ...s, spindleRpm })),
        ...staleGcode(),
      })),
    setOperationType: (id, type) =>
      set((s) => ({
        operations: s.operations.map((op) => {
          if (op.id !== id) return op;
          const kind: OperationKind =
            type === "cncRelief"
              ? DEFAULT_RELIEF
              : type === "cncCutout"
                ? defaultCutout(s.thickness)
              : type === "cncProfile"
                ? defaultOperation("cnc").kind
                : type === "laserImage"
                  ? { type, interval: 0.15, dither: true }
                  : type === "laserFill"
                    ? { type, interval: 0.1 }
                    : { type: "laserCut" };
          // The old bit may not suit the new job, so a fresh one is suggested.
          return equip({ ...op, kind, toolId: null }, s);
        }),
        ...staleGcode(),
      })),
    setOperationTool: (id, toolId) =>
      set((s) => ({
        operations: s.operations.map((op) => (op.id === id ? equip({ ...op, toolId }, s) : op)),
        ...staleGcode(),
      })),

    updateTrace: (patch) => {
      set((s) => ({
        trace: { ...s.trace, settings: { ...s.trace.settings, ...patch } },
        ...staleGcode(),
      }));
      scheduleTrace();
    },

    updatePreprocess: (patch) => {
      const { settings } = get().trace;
      get().updateTrace({ preprocess: { ...settings.preprocess, ...patch } });
    },

    exportSvg: async () => {
      const { image, trace, widthMm, heightMm } = get();
      if (!image) return;
      const suggestedName = `${image.name.replace(/\.[^.]+$/, "")}.svg`;
      try {
        const path = await ipc.exportSvg(trace.settings, widthMm, heightMm, suggestedName);
        if (path) set((s) => ({ exportedTo: path, trace: { ...s.trace, error: null } }));
      } catch (e) {
        set((s) => ({ trace: { ...s.trace, error: message(e) } }));
      }
    },

    addOperation: () =>
      set((s) => ({
        operations: [...s.operations, fresh(s.machineKind, s)],
        ...staleGcode(),
      })),

    addCutout: () =>
      set((s) => ({
        operations: [
          ...s.operations,
          equip(
            { id: nextOperationId++, color: null, kind: defaultCutout(s.thickness), feed: 600, power: 0, passes: 1 },
            s,
          ),
        ],
        ...staleGcode(),
      })),

    updateOperation: (id, patch) =>
      set((s) => ({
        operations: s.operations.map((op) => (op.id === id ? { ...op, ...patch } : op)),
        ...staleGcode(),
      })),

    removeOperation: (id) =>
      set((s) => ({ operations: s.operations.filter((op) => op.id !== id), ...staleGcode() })),

    generateGcode: async () => {
      const s = get();
      const rect = artworkRect(s);
      if (!rect) return;
      const generation = ++gcodeGeneration;
      set({ gcode: { ...EMPTY_GCODE, busy: true } });
      try {
        const program = await ipc.generateGcode(s.trace.settings, s, {
          machine: {
            kind: s.machineKind,
            origin: s.origin,
            startGcode: s.startGcode,
            endGcode: s.endGcode,
            bedWidth: s.bed.width,
            bedHeight: s.bed.height,
            maxPowerS: s.maxPowerS,
            travelFeed: TRAVEL_FEED,
          },
          offset: [rect.x, rect.y],
          operations: s.operations.filter((op) => op.enabled !== false),
        }, {
          enabled: s.heightSource === "depth" && s.depth.status === "ready",
          detail: s.detail,
          floor: s.depthFloor,
        });
        if (generation !== gcodeGeneration) return;
        set({ gcode: { ...EMPTY_GCODE, program } });
        usePlayback.getState().finish();
        // Keep the project once it has produced something, after the preview
        // has had a moment to draw so the thumbnail shows the result.
        setTimeout(() => void get().saveProject(), 800);
      } catch (e) {
        if (generation !== gcodeGeneration) return;
        set({ gcode: { ...EMPTY_GCODE, error: message(e) } });
      }
    },

    exportGcode: async () => {
      const { image, gcode } = get();
      if (!image || !gcode.program) return;
      const generation = gcodeGeneration;
      const suggestedName = `${image.name.replace(/\.[^.]+$/, "")}.gcode`;
      try {
        const path = await ipc.exportGcode(suggestedName);
        if (path && generation === gcodeGeneration) {
          set((s) => ({ gcode: { ...s.gcode, exportedTo: path, error: null } }));
        }
      } catch (e) {
        set((s) => ({ gcode: { ...s.gcode, error: message(e) } }));
      }
    },
  };
});

// In simple mode the setup follows the machine and the material's thickness:
// a different machine offers different goals, and depths depend on thickness.
useProject.subscribe((s, previous) => {
  if (s.mode !== "simple") return;
  if (s.machineKind !== previous.machineKind || s.thickness !== previous.thickness) s.setSimple({});
});

// Remember the machine between sessions.
useProject.subscribe((s, previous) => {
  const keys = ["machinePreset", "machineKind", "bed", "origin", "maxPowerS", "spindleRpm", "startGcode", "endGcode"] as const;
  if (keys.every((key) => s[key] === previous[key])) return;
  try {
    localStorage.setItem(DEVICE_KEY, JSON.stringify(Object.fromEntries(keys.map((key) => [key, s[key]]))));
  } catch {
    // Not persisted; the choice still applies for this session.
  }
});

// Record changes for undo.
useProject.subscribe((s, previous) => {
  if (stepping) return;
  const before = snapshot(previous);
  if (sameSnapshot(before, snapshot(s))) return;
  if (s.image !== previous.image) {
    // A different picture, or a crop, starts a new history: the earlier
    // settings belonged to an image that is no longer there.
    past.length = 0;
    future.length = 0;
  } else {
    const now = Date.now();
    if (now - lastChange > SAME_STEP_MS) {
      past.push(before);
      if (past.length > HISTORY_LIMIT) past.shift();
    }
    lastChange = now;
    future.length = 0;
  }
  const flags = { canUndo: past.length > 0, canRedo: future.length > 0 };
  if (flags.canUndo !== s.canUndo || flags.canRedo !== s.canRedo) useProject.setState(flags);
});

// Ctrl+Z and Ctrl+Y (or Ctrl+Shift+Z), except while typing in a field, where
// they keep their usual meaning of undoing the typing.
window.addEventListener("keydown", (e) => {
  if (!(e.ctrlKey || e.metaKey) || e.altKey) return;
  const target = e.target as HTMLElement | null;
  if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA") && (target as HTMLInputElement).type !== "range" && (target as HTMLInputElement).type !== "checkbox") return;
  const key = e.key.toLowerCase();
  if (key === "z" && !e.shiftKey) {
    e.preventDefault();
    useProject.getState().undo();
  } else if (key === "y" || (key === "z" && e.shiftKey)) {
    e.preventDefault();
    useProject.getState().redo();
  }
});
