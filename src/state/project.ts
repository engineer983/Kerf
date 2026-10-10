import { create } from "zustand";
import { isAcceptedImage, loadImageFile } from "../lib/image";
import { estimateDepth } from "../lib/depth";
import * as ipc from "../lib/ipc";
import { CUSTOM_PRESET, MACHINE_PRESETS } from "../lib/machines";
import { cutSettings } from "../lib/feeds";
import { DEFAULT_MATERIAL, findMaterial, type Material } from "../lib/materials";
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
};

let nextOperationId = 1;

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
  setStep: (step: Step) => void;
  importFile: (file: File) => Promise<void>;
  clearImage: () => void;
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
  /** Runs the depth model on the image. Downloads the model the first time. */
  estimateDepth: () => Promise<void>;
  setPosition: (patch: { left?: number; front?: number }) => void;
  setBed: (patch: { width?: number; height?: number }) => void;
  setMachineKind: (kind: MachineKind) => void;
  setMaxPowerS: (maxPowerS: number) => void;
  setOrigin: (origin: Origin) => void;
  applyMachinePreset: (id: string) => void;
  setShowSource: (show: boolean) => void;
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
  updateOperation: (id: number, patch: Partial<Operation>) => void;
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

const DEVICE_KEY = "kerf.device";

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
    if (op.kind.type !== "cncRelief" && op.kind.type !== "cncProfile") return op;
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

  /** Drops any generated G-code; call when one of its inputs changes. */
  function staleGcode(): { gcode: GcodeState } {
    gcodeGeneration++;
    return { gcode: EMPTY_GCODE };
  }

  // A machine chosen in an earlier session is used again; otherwise the app
  // opens on the Device step so that one is chosen first.
  const remembered = storedDevice();

  return {
    step: remembered ? "import" : "device",
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
    detail: 0.3,
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

    setStep: (step) => set({ step }),

    importFile: async (file) => {
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
          heightSource: "brightness" as const,
          depth: NO_DEPTH,
          left: STOCK_MARGIN_MM,
          front: STOCK_MARGIN_MM,
          stockWidth: widthMm + STOCK_MARGIN_MM * 2,
          stockLength: Math.ceil(heightMm) + STOCK_MARGIN_MM * 2,
        };
        set({
          ...placed,
          operations: [fresh(machineKind, { ...get(), ...placed })],
          error: null,
          exportedTo: null,
          trace: { ...EMPTY_TRACE, settings: trace.settings },
          step: "trace",
          ...staleGcode(),
        });
        scheduleTrace();
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
          operations: s.operations,
        }, { use: s.heightSource === "depth" && s.depth.status === "ready", detail: s.detail });
        if (generation !== gcodeGeneration) return;
        set({ gcode: { ...EMPTY_GCODE, program } });
        usePlayback.getState().finish();
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
