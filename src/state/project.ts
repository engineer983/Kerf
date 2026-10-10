import { create } from "zustand";
import { isAcceptedImage, loadImageFile } from "../lib/image";
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

export type Step = "import" | "trace" | "operations" | "machine";

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
        ballNose: tool.kind === "ball",
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
  /** Physical width of the artwork in millimetres; height follows the aspect ratio. */
  widthMm: number;
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

function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** Where the artwork sits on the bed, in bed coordinates (mm, Y down from the back). */
export function artworkRect(
  s: Pick<ProjectState, "image" | "widthMm" | "left" | "front" | "bed" | "origin">,
) {
  if (!s.image) return null;
  const height = (s.widthMm * s.image.height) / s.image.width;
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

  return {
    step: "import",
    image: null,
    bed: { width: 400, height: 400 },
    machineKind: "laser",
    maxPowerS: 1000,
    machinePreset: CUSTOM_PRESET,
    origin: "frontLeft",
    startGcode: "",
    endGcode: "",
    widthMm: 100,
    left: STOCK_MARGIN_MM,
    front: STOCK_MARGIN_MM,
    stockWidth: 300,
    stockLength: 200,
    spindleRpm: 16000,
    showSource: true,
    materialId: DEFAULT_MATERIAL.id,
    thickness: DEFAULT_MATERIAL.thickness,
    error: null,
    trace: EMPTY_TRACE,
    exportedTo: null,
    operations: [defaultOperation("laser")],
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
        const scale = Math.min((bed.width * 0.5) / image.width, (bed.height * 0.5) / image.height);
        const widthMm = Math.round(image.width * scale);
        const heightMm = (widthMm * image.height) / image.width;
        const placed = {
          image,
          widthMm,
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

    setWidthMm: (widthMm) => set({ widthMm, ...staleGcode() }),
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
      const { image, trace, widthMm } = get();
      if (!image) return;
      const suggestedName = `${image.name.replace(/\.[^.]+$/, "")}.svg`;
      try {
        const path = await ipc.exportSvg(trace.settings, widthMm, suggestedName);
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
        const program = await ipc.generateGcode(s.trace.settings, s.widthMm, {
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
        });
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
