import type { OperationKind } from "./ipc";
import { findMaterial, MATERIALS } from "./materials";
import { TOOL_KIND_LABELS } from "./tools";
import { artworkRect, stockRect, useProject } from "../state/project";
import { useToolbox } from "../state/tools";

// What the assistant may do. Every tool reads or changes the project through
// the same actions the interface uses, so its changes can be undone like any
// other. There is deliberately no tool that connects to, moves or starts a
// machine: running a job is always a person's decision.

export interface ToolSpec {
  name: string;
  description: string;
  schema: Record<string, unknown>;
}

export interface ToolOutcome {
  /** What the model is told. */
  content: string;
  isError: boolean;
  /** A short line for the person, shown in the conversation. */
  summary: string;
}

const OPERATION_TYPES = ["laserImage", "laserFill", "laserCut", "cncProfile", "cncRelief", "cncCutout"] as const;

const number = (description: string) => ({ type: "number", description });

export const TOOLS: ToolSpec[] = [
  {
    name: "get_project",
    description:
      "Read the current project: machine, material, artwork size and position, trace settings, operations with their settings, the bits in the toolbox, and the statistics of the last generated program. Call this before suggesting or changing anything, since the person may have changed things since you last looked.",
    schema: { type: "object", properties: {} },
  },
  {
    name: "update_trace",
    description:
      "Change how the picture is prepared. Brightness, contrast and blur also shape image engravings and reliefs. Only the fields given are changed.",
    schema: {
      type: "object",
      properties: {
        mode: { type: "string", enum: ["outline", "color"], description: "Trace dark areas as shapes, or one set of shapes per colour." },
        brightness: number("-1 (darker) to 1 (lighter); 0 is unchanged."),
        contrast: number("-1 (flatter) to 1 (stronger); 0 is unchanged."),
        blur: number("Blur radius in pixels, 0 to 5."),
        threshold: { description: 'Dark/light dividing level for outlines: "auto", or 0 to 255.', anyOf: [{ type: "string", enum: ["auto"] }, { type: "number" }] },
        invert: { type: "boolean", description: "Trace the light areas instead of the dark ones." },
        ignoreSpecks: number("Drop shapes smaller than this many pixels across, 0 to 16."),
        smoothCurves: { type: "boolean" },
      },
    },
  },
  {
    name: "set_material",
    description: "Change the material and the size of the piece. Changing the material recalculates router speeds.",
    schema: {
      type: "object",
      properties: {
        material: { type: "string", enum: MATERIALS.map((m) => m.id) },
        thickness: number("Millimetres."),
        width: number("Left to right, millimetres."),
        length: number("Front to back, millimetres."),
      },
    },
  },
  {
    name: "set_artwork",
    description: "Resize or move the artwork on the material. Sizes and distances are in millimetres.",
    schema: {
      type: "object",
      properties: {
        width: number("Artwork width."),
        height: number("Artwork height."),
        fromLeft: number("Distance of the artwork's left edge from the left of the bed."),
        fromEdge: number("Distance from the bed edge on the machine's origin side (front for most routers)."),
        keepProportions: { type: "boolean", description: "Whether width and height change together." },
      },
    },
  },
  {
    name: "set_operation",
    description:
      "Change one operation's settings. Only the fields given are changed, and fields that do not apply to the operation's type are ignored. Changing `type` or `bit` resets speeds to suit, so set those first and other fields after.",
    schema: {
      type: "object",
      properties: {
        operation: number("Which operation, counting from 1."),
        type: { type: "string", enum: [...OPERATION_TYPES], description: "laser*: laser machines only. cnc*: routers only." },
        bit: { type: "string", description: "A bit's id or name from the toolbox (router operations)." },
        include: { type: "boolean", description: "Whether the operation is part of the generated job." },
        speed: number("Cutting or engraving speed, mm/min."),
        power: number("Laser power, 0 to 100 percent."),
        passes: number("Laser repeats, 1 or more."),
        lineInterval: number("Gap between engraved lines, mm (laser fill and image)."),
        shading: { type: "string", enum: ["dots", "power"], description: "Laser image: dithered dots, or varying power." },
        depth: number("Deepest cut, mm."),
        depthPerPass: number("mm."),
        plungeSpeed: number("mm/min."),
        safeHeight: number("mm above the material."),
        stepover: number("Gap between rows of a relief, mm."),
        raisedAreas: { type: "string", enum: ["light", "dark"], description: "Relief: which parts of the picture stand proud." },
        offset: number("Cutout: distance of the cut line outside the artwork, mm; negative is inside."),
        cutSide: { type: "string", enum: ["outside", "on", "inside"] },
        shape: { type: "string", enum: ["rectangle", "ellipse"] },
        cornerRadius: number("Cutout rectangle corner radius, mm."),
        tabs: number("Cutout: number of holding tabs; 0 for none."),
        tabWidth: number("mm."),
        tabHeight: number("mm."),
      },
      required: ["operation"],
    },
  },
  {
    name: "add_operation",
    description: "Add an operation at the end of the job. Returns its number.",
    schema: {
      type: "object",
      properties: { type: { type: "string", enum: [...OPERATION_TYPES] } },
      required: ["type"],
    },
  },
  {
    name: "remove_operation",
    description: "Remove an operation.",
    schema: { type: "object", properties: { operation: number("Which operation, counting from 1.") }, required: ["operation"] },
  },
  {
    name: "set_relief_height",
    description:
      'Choose what a relief takes its heights from. "depth" uses a model\'s estimate of how near each part of the picture is, which suits photographs and drawings of objects; choosing it runs the estimate if it has not been run, which can take a while the first time.',
    schema: {
      type: "object",
      properties: {
        source: { type: "string", enum: ["brightness", "depth"] },
        pictureDetail: number("0 to 1: how strongly the picture's own linework is cut into a depth relief."),
        flattenBackground: number("0 to 0.9: how much of the far background is levelled off."),
      },
    },
  },
  {
    name: "new_project",
    description:
      "Start a new, empty project. The current project is saved first, so nothing is lost. Afterwards there is no picture: either the person imports one, or you draw one with create_picture.",
    schema: { type: "object", properties: {} },
  },
  {
    name: "create_picture",
    description:
      "Draw a picture yourself, as SVG, and load it as the project's picture. Use this when the person asks you to make or draw something (a flag, an emblem, lettering, a simple illustration or pattern) rather than supplying a picture. It replaces the current picture and starts a new project. You can only draw what SVG can express; you cannot produce a photograph. For a 3D carving, draw a height map and set heightMap to true: white is the highest point and black the lowest, and smooth gradients between them become smooth slopes, so rolling waves are bands of light and dark gradient, and a raised border is a lighter band. Detail drawn in flat colour with no shading carves flat. For laser engraving or cutting, draw normally in black on white. Give the SVG a viewBox and draw at the proportions of the finished piece.",
    schema: {
      type: "object",
      properties: {
        name: { type: "string", description: "A short name for the picture, such as 'wavy flag'." },
        svg: { type: "string", description: "A complete SVG document, starting with <svg and including a viewBox." },
        heightMap: { type: "boolean", description: "True if the picture is a greyscale height map for carving: brightness is height." },
      },
      required: ["name", "svg"],
    },
  },
  {
    name: "generate_gcode",
    description:
      "Work out the tool's moves for the included operations and return the job's statistics (time, distances, extent), or the reason it could not be generated. This does not send anything to a machine.",
    schema: { type: "object", properties: {} },
  },
];

class ToolError extends Error {}

/** The most SVG the assistant may send as one drawing. */
const MAX_SVG_CHARS = 400_000;
/** Longest side, in pixels, of the picture made from a drawing. */
const DRAWN_PICTURE_PX = 1600;

type Input = Record<string, unknown>;

/** A number from the model's input, checked; undefined when the field was not given. */
function num(input: Input, key: string, min: number, max: number): number | undefined {
  const value = input[key];
  if (value === undefined || value === null) return undefined;
  if (typeof value !== "number" || !Number.isFinite(value)) throw new ToolError(`${key} must be a number.`);
  if (value < min || value > max) throw new ToolError(`${key} must be between ${min} and ${max}.`);
  return value;
}

function choice<T extends string>(input: Input, key: string, options: readonly T[]): T | undefined {
  const value = input[key];
  if (value === undefined || value === null) return undefined;
  if (typeof value !== "string" || !options.includes(value as T)) {
    throw new ToolError(`${key} must be one of: ${options.join(", ")}.`);
  }
  return value as T;
}

function flag(input: Input, key: string): boolean | undefined {
  const value = input[key];
  if (value === undefined || value === null) return undefined;
  if (typeof value !== "boolean") throw new ToolError(`${key} must be true or false.`);
  return value;
}

/** The operation the model named, counting from 1. */
function operationAt(input: Input) {
  const { operations } = useProject.getState();
  const index = num(input, "operation", 1, operations.length);
  if (index === undefined || !Number.isInteger(index)) {
    throw new ToolError(`operation must be a whole number from 1 to ${operations.length}.`);
  }
  return operations[index - 1];
}

function describeProject() {
  const s = useProject.getState();
  const { tools } = useToolbox.getState();
  const art = artworkRect(s);
  const stock = stockRect(s);
  const { preprocess } = s.trace.settings;
  return {
    machine: {
      kind: s.machineKind === "cnc" ? "CNC router" : "laser",
      preset: s.machinePreset,
      bedMm: s.bed,
      origin: s.origin,
      routerRpm: s.machineKind === "cnc" ? s.spindleRpm : undefined,
    },
    picture: s.image
      ? { name: s.image.name, pixels: [s.image.width, s.image.height], looksLikeAPhoto: s.image.photo, cropped: s.uncropped !== null }
      : null,
    material: { id: s.materialId, name: findMaterial(s.materialId).label, thicknessMm: s.thickness, widthMm: s.stockWidth, lengthMm: s.stockLength },
    artwork: art && {
      widthMm: s.widthMm,
      heightMm: s.heightMm,
      fromLeftMm: s.left,
      fromEdgeMm: s.front,
      keepProportions: s.aspectLocked,
      fitsOnMaterial:
        art.x >= stock.x - 0.01 && art.y >= stock.y - 0.01 && art.x + art.width <= stock.x + stock.width + 0.01 && art.y + art.height <= stock.y + stock.height + 0.01,
    },
    trace: {
      mode: s.trace.settings.mode,
      brightness: preprocess.brightness,
      contrast: preprocess.contrast,
      blur: preprocess.blurSigma,
      threshold: preprocess.threshold.mode === "otsu" ? "auto" : preprocess.threshold.level,
      invert: preprocess.invert,
      ignoreSpecks: s.trace.settings.speckle,
      smoothCurves: s.trace.settings.curves,
      tracedPaths: s.trace.result?.pathCount ?? null,
    },
    reliefHeight: { source: s.heightSource, depthEstimated: s.depth.status === "ready", pictureDetail: s.detail, flattenBackground: s.depthFloor },
    operations: s.operations.map((op, i) => ({
      operation: i + 1,
      include: op.enabled !== false,
      bit: tools.find((t) => t.id === op.toolId)?.name ?? null,
      speedMmPerMin: op.feed,
      ...(op.kind.type.startsWith("laser") ? { powerPercent: op.power, passes: op.passes } : {}),
      ...op.kind,
    })),
    toolbox: tools.map((t) => ({
      id: t.id,
      name: t.name,
      kind: TOOL_KIND_LABELS[t.kind],
      diameterMm: t.diameter,
      flutes: t.flutes,
      cuttingLengthMm: t.cutLength,
    })),
    materialsAvailable: MATERIALS.map((m) => ({ id: m.id, name: m.label, canBeRouted: m.machining !== null })),
    lastGenerated: s.gcode.program && {
      estimatedMinutes: Math.round(s.gcode.program.seconds / 60),
      cuttingMetres: Math.round(s.gcode.program.cutMm / 10) / 100,
      lines: s.gcode.program.lineCount,
      extentMm: s.gcode.program.bounds,
    },
    notGeneratedBecause: s.gcode.error,
  };
}

const HANDLERS: Record<string, (input: Input) => Promise<Omit<ToolOutcome, "isError">> | Omit<ToolOutcome, "isError">> = {
  get_project: () => ({ content: JSON.stringify(describeProject()), summary: "Looked at the project" }),

  update_trace: (input) => {
    const s = useProject.getState();
    const mode = choice(input, "mode", ["outline", "color"] as const);
    const brightness = num(input, "brightness", -1, 1);
    const contrast = num(input, "contrast", -1, 1);
    const blur = num(input, "blur", 0, 5);
    const invert = flag(input, "invert");
    const speckle = num(input, "ignoreSpecks", 0, 16);
    const curves = flag(input, "smoothCurves");
    const threshold =
      input.threshold === undefined || input.threshold === null
        ? undefined
        : input.threshold === "auto"
          ? ({ mode: "otsu" } as const)
          : ({ mode: "manual", level: Math.round(num(input, "threshold", 0, 255)!) } as const);

    const preprocess = {
      ...s.trace.settings.preprocess,
      ...(brightness !== undefined && { brightness }),
      ...(contrast !== undefined && { contrast }),
      ...(blur !== undefined && { blurSigma: blur }),
      ...(invert !== undefined && { invert }),
      ...(threshold && { threshold }),
    };
    s.updateTrace({
      preprocess,
      ...(mode && { mode }),
      ...(speckle !== undefined && { speckle: Math.round(speckle) }),
      ...(curves !== undefined && { curves }),
    });
    return { content: "Trace settings updated.", summary: "Changed the trace settings" };
  },

  set_material: (input) => {
    const s = useProject.getState();
    const material = choice(input, "material", MATERIALS.map((m) => m.id));
    const thickness = num(input, "thickness", 0.1, 200);
    const width = num(input, "width", 1, 5000);
    const length = num(input, "length", 1, 5000);
    if (material) s.setMaterial(material);
    if (thickness !== undefined) s.setThickness(thickness);
    if (width !== undefined || length !== undefined) {
      s.setStock({ ...(width !== undefined && { stockWidth: width }), ...(length !== undefined && { stockLength: length }) });
    }
    return {
      content: "Material updated. Router speeds were recalculated if the material changed.",
      summary: material ? `Set the material to ${findMaterial(material).label}` : "Changed the material's size",
    };
  },

  set_artwork: (input) => {
    const s = useProject.getState();
    if (!s.image) throw new ToolError("There is no picture loaded yet.");
    const keep = flag(input, "keepProportions");
    const width = num(input, "width", 1, 5000);
    const height = num(input, "height", 1, 5000);
    const left = num(input, "fromLeft", -5000, 5000);
    const front = num(input, "fromEdge", -5000, 5000);
    if (keep !== undefined) s.setAspectLocked(keep);
    if (width !== undefined) s.setWidthMm(width);
    if (height !== undefined) useProject.getState().setHeightMm(height);
    if (left !== undefined || front !== undefined) {
      s.setPosition({ ...(left !== undefined && { left }), ...(front !== undefined && { front }) });
    }
    const now = useProject.getState();
    return {
      content: `Artwork is now ${now.widthMm} x ${now.heightMm} mm, ${now.left} mm from the left and ${now.front} mm from the edge.`,
      summary: "Resized or moved the artwork",
    };
  },

  set_operation: (input) => {
    const target = operationAt(input);
    const actions = useProject.getState();
    const type = choice(input, "type", OPERATION_TYPES);
    if (type && type !== target.kind.type) {
      if (type.startsWith("laser") !== (actions.machineKind === "laser")) {
        throw new ToolError(`A ${actions.machineKind === "laser" ? "laser" : "router"} cannot run a ${type} operation.`);
      }
      actions.setOperationType(target.id, type);
    }
    if (typeof input.bit === "string") {
      const wanted = input.bit.toLowerCase();
      const bit = useToolbox.getState().tools.find((t) => t.id === input.bit || t.name.toLowerCase() === wanted);
      if (!bit) throw new ToolError(`No bit called "${input.bit}" is in the toolbox.`);
      actions.setOperationTool(target.id, bit.id);
    }

    // Read the operation again: changing its type or bit replaced its settings.
    const op = useProject.getState().operations.find((o) => o.id === target.id)!;
    const kind = { ...op.kind } as Record<string, unknown> & { type: OperationKind["type"] };
    const set = (field: string, value: unknown) => {
      if (value !== undefined && field in kind) kind[field] = value;
    };
    set("interval", num(input, "lineInterval", 0.02, 10));
    const shading = choice(input, "shading", ["dots", "power"] as const);
    set("dither", shading && shading === "dots");
    set("depth", num(input, "depth", 0.01, 200));
    set("depthPerPass", num(input, "depthPerPass", 0.01, 200));
    set("plungeFeed", num(input, "plungeSpeed", 1, 100000));
    set("safeZ", num(input, "safeHeight", 0.1, 200));
    set("stepover", num(input, "stepover", 0.02, 50));
    const raised = choice(input, "raisedAreas", ["light", "dark"] as const);
    set("lightIsHigh", raised && raised === "light");
    set("offset", num(input, "offset", -1000, 1000));
    set("side", choice(input, "cutSide", ["outside", "on", "inside"] as const));
    set("outline", choice(input, "shape", ["rectangle", "ellipse"] as const));
    set("cornerRadius", num(input, "cornerRadius", 0, 1000));
    const tabs = num(input, "tabs", 0, 40);
    set("tabs", tabs === undefined ? undefined : Math.round(tabs));
    set("tabWidth", num(input, "tabWidth", 0.5, 100));
    set("tabHeight", num(input, "tabHeight", 0.1, 50));

    const speed = num(input, "speed", 1, 100000);
    const power = num(input, "power", 0, 100);
    const passes = num(input, "passes", 1, 100);
    const include = flag(input, "include");
    actions.updateOperation(op.id, {
      kind: kind as OperationKind,
      ...(speed !== undefined && { feed: speed }),
      ...(power !== undefined && { power }),
      ...(passes !== undefined && { passes: Math.round(passes) }),
      ...(include !== undefined && { enabled: include }),
    });
    const number = useProject.getState().operations.findIndex((o) => o.id === op.id) + 1;
    return { content: `Operation ${number} updated.`, summary: `Changed operation ${number}` };
  },

  add_operation: (input) => {
    const type = choice(input, "type", OPERATION_TYPES);
    if (!type) throw new ToolError("type is required.");
    const s = useProject.getState();
    if (type.startsWith("laser") !== (s.machineKind === "laser")) {
      throw new ToolError(`A ${s.machineKind === "laser" ? "laser" : "router"} cannot run a ${type} operation.`);
    }
    if (type === "cncCutout") s.addCutout();
    else {
      s.addOperation();
      const added = useProject.getState().operations.at(-1)!;
      if (added.kind.type !== type) useProject.getState().setOperationType(added.id, type);
    }
    const count = useProject.getState().operations.length;
    return { content: `Added as operation ${count}.`, summary: `Added operation ${count}` };
  },

  remove_operation: (input) => {
    const target = operationAt(input);
    const s = useProject.getState();
    if (s.operations.length === 1) throw new ToolError("The last operation cannot be removed; change its type instead.");
    s.removeOperation(target.id);
    return { content: "Removed.", summary: "Removed an operation" };
  },

  set_relief_height: async (input) => {
    const s = useProject.getState();
    const source = choice(input, "source", ["brightness", "depth"] as const);
    const detail = num(input, "pictureDetail", 0, 1);
    const floor = num(input, "flattenBackground", 0, 0.9);
    if (detail !== undefined) s.setDetail(detail);
    if (floor !== undefined) s.setDepthFloor(floor);
    if (source === "brightness") s.setHeightSource("brightness");
    if (source === "depth") {
      if (!s.image) throw new ToolError("There is no picture loaded yet.");
      if (useProject.getState().depth.status !== "ready") await useProject.getState().estimateDepth();
      const depth = useProject.getState().depth;
      if (depth.status !== "ready") throw new ToolError(`The depth could not be estimated: ${depth.message ?? "unknown reason"}`);
      useProject.getState().setHeightSource("depth");
    }
    return { content: "Relief height settings updated.", summary: "Changed where the relief takes its height from" };
  },

  new_project: async () => {
    await useProject.getState().newProject();
    return {
      content: "A new, empty project is open. There is no picture yet: the person can import one, or you can draw one with create_picture.",
      summary: "Started a new project",
    };
  },

  create_picture: async (input) => {
    const name = typeof input.name === "string" && input.name.trim() ? input.name.trim().slice(0, 60) : "drawing";
    const heightMap = flag(input, "heightMap") ?? false;
    if (typeof input.svg !== "string") throw new ToolError("svg must be a string containing an SVG document.");
    let svg = input.svg.trim();
    const start = svg.indexOf("<svg");
    if (start < 0) throw new ToolError("svg must contain an <svg> element.");
    svg = svg.slice(start);
    if (svg.length > MAX_SVG_CHARS) throw new ToolError(`The SVG is too large (${svg.length} characters; the limit is ${MAX_SVG_CHARS}). Simplify it.`);
    // An SVG shown as an image must declare its namespace.
    if (!/<svg[^>]*xmlns=/.test(svg)) svg = svg.replace("<svg", '<svg xmlns="http://www.w3.org/2000/svg"');

    // The drawing is turned into an ordinary PNG. Loading it as an image means
    // nothing inside it can run, and nothing it refers to is fetched.
    const url = URL.createObjectURL(new Blob([svg], { type: "image/svg+xml" }));
    let png: Blob;
    try {
      const drawing = new Image();
      drawing.src = url;
      try {
        await drawing.decode();
      } catch {
        throw new ToolError("The SVG could not be drawn. Check that it is well-formed and has a viewBox.");
      }
      const [w, h] = [drawing.naturalWidth || 300, drawing.naturalHeight || 150];
      const scale = DRAWN_PICTURE_PX / Math.max(w, h);
      const canvas = document.createElement("canvas");
      canvas.width = Math.max(1, Math.round(w * scale));
      canvas.height = Math.max(1, Math.round(h * scale));
      const ctx = canvas.getContext("2d")!;
      // Undrawn areas are white: untouched for a laser. For a height map, the
      // drawing is expected to cover the whole picture.
      ctx.fillStyle = "#fff";
      ctx.fillRect(0, 0, canvas.width, canvas.height);
      ctx.drawImage(drawing, 0, 0, canvas.width, canvas.height);
      const made = await new Promise<Blob | null>((done) => canvas.toBlob(done, "image/png"));
      if (!made) throw new ToolError("The drawing could not be turned into a picture.");
      png = made;
    } finally {
      URL.revokeObjectURL(url);
    }

    const before = useProject.getState();
    // What was being worked on is kept before it is replaced.
    if (before.image) await before.saveProject();
    await useProject.getState().importFile(new File([png], `${name}.png`, { type: "image/png" }), { drawnHeightMap: heightMap });
    const after = useProject.getState();
    if (after.error || !after.image || after.image === before.image) {
      throw new ToolError(after.error ?? "The picture could not be loaded.");
    }
    return {
      content: `The picture "${name}" is loaded (${after.image.width} x ${after.image.height} pixels) as a new project${heightMap ? ", and a carving will take its height from brightness" : ""}. Look at the project to see how it was set up.`,
      summary: `Drew a picture: ${name}`,
    };
  },

  generate_gcode: async () => {
    const s = useProject.getState();
    if (!s.image) throw new ToolError("There is no picture loaded yet.");
    await s.generateGcode();
    const { program, error } = useProject.getState().gcode;
    if (!program) throw new ToolError(error ?? "The program was not generated.");
    return {
      content: JSON.stringify({
        estimatedMinutes: Math.round(program.seconds / 60),
        cuttingMetres: Math.round(program.cutMm / 10) / 100,
        travelMetres: Math.round(program.travelMm / 10) / 100,
        lines: program.lineCount,
        extentMm: program.bounds,
      }),
      summary: "Generated the G-code",
    };
  },
};

/** Runs a tool the model called. Failures are reported to the model, not thrown. */
export async function runTool(name: string, input: unknown): Promise<ToolOutcome> {
  const handler = HANDLERS[name];
  if (!handler) return { content: `There is no tool called ${name}.`, isError: true, summary: `Tried an unknown action (${name})` };
  if (typeof input !== "object" || input === null || "_unreadable" in input) {
    return { content: "The tool's input was not a valid JSON object.", isError: true, summary: `Could not read the input for ${name}` };
  }
  try {
    return { ...(await handler(input as Input)), isError: false };
  } catch (e) {
    const message = e instanceof Error ? e.message : String(e);
    return { content: message, isError: true, summary: `Could not do that: ${message}` };
  }
}

/** Standing instructions for the assistant. Kept unchanging so providers can cache it. */
export const SYSTEM_PROMPT = `You are the assistant inside Kerf, a desktop app that turns a picture into a job for a laser engraver or a CNC router. The person you are helping makes things in a home workshop. They may be new to this, so explain in plain language and define a term the first time you use it.

How the app works: the person picks a machine, imports a picture, and the app either traces it into outlines or uses it directly. In Operations they choose a material, place the artwork on it, and add operations. A laser can engrave the picture with its shading (image), fill traced shapes, or follow their outlines. A router can follow traced outlines (profile), carve the picture as a 3D relief, or cut the piece out with holding tabs. A relief takes its heights either from the picture's brightness or from an estimate of depth, which suits photographs and drawings of objects. Generating produces G-code and a preview; nothing reaches a machine until the person sends it themselves.

You can read the project and change its settings with your tools. Look at the project before advising or changing anything, since it may have changed since you last looked. When the person asks for a result ("make it deeper", "it's taking too long"), make the change rather than describing how they could, then say in a sentence or two what you changed and why. Your changes can be undone with Ctrl+Z, so prefer acting to asking, but do ask when the request could reasonably mean quite different things. After changing something that affects the job, generate the G-code again if they will want to see the result, and report the new time.

Most requests are a wish, not a setting: "make it as realistic as possible", "I want this on a cutting board", "it takes too long". Your job is to turn the wish into a complete, sensible setup, setting every relevant thing yourself in one go rather than adjusting one number and leaving the rest. Work out what result they are after, look at the project, make all the changes, generate, and then report: what you set up and why in a few sentences, how long the job will take, and the one or two trade-offs worth knowing. If the job comes out very long, say so and offer the faster alternative you would choose. Ask a question first only when something essential is unknown and guessing wrong would waste material, such as the material's real thickness when a cut-through depends on it. Otherwise state your assumption and proceed.

What you know about getting good results in this app:

Realism and fine detail on a router come from a 3D relief. For a photograph or a drawing of an object, take the height from estimated depth, since brightness is not shape. The depth estimate gives a smooth form, so add a little of the picture's own detail back. A small amount goes a long way: start around 0.1 to 0.2 for line art, logos and lettering, and 0.05 to 0.15 for photographs of people, where more turns skin rough. Raise it only if the person asks for crisper linework, and look at the result rather than assuming more is better. Flatten the background by 0.2 to 0.4 when the subject sits on a backdrop, so the subject uses the full depth. Detail is limited by the bit's tip: choose the smallest ball nose or tapered ball nose in the toolbox that the person will wait for, and keep the stepover to about a quarter of a tapered bit's tip diameter, or a tenth to an eighth of a plain ball nose's diameter. More depth reads as more realism, typically 3 to 6 mm, but leave a few millimetres of material under the deepest cut and stay within the bit's cutting length. A larger piece also shows more detail for the same bit.

Realism on a laser comes from image engraving, which keeps shading; fill and line operations are flat and suit logos and text. Dots (dithering) suit wood and other materials that do not darken evenly with power. A line interval near 0.1 mm is about as fine as a diode laser's spot allows; coarser is faster and lighter. A little extra contrast usually helps a photograph. You do not know the right power and speed for their laser and material, so say so and suggest a small test grid on scrap.

Speed is the opposite trade: a larger stepover or line interval, a larger bit, less depth, or a smaller piece. Say what detail is given up.

Clean signs and logos: on a laser, fill the traced shapes; on a router, follow the traced paths, or carve a relief from brightness with some blur for a smooth, embossed look.

To free the finished piece on a router, add a cutout last: outside the line, depth equal to the material's thickness, with tabs so the piece cannot come loose. Operations that use different bits cannot run as one job, because the bit must be changed and the height set again in between; include one at a time and tell the person the order to run them in.

Before you finish, check that the artwork fits on the material, that no depth exceeds the bit's cutting length, and that a relief does not go through the material.

You can start a new project, and you can make the picture yourself by drawing it as SVG. When the person asks you to create something rather than giving you a picture, draw it: start a new project if one is open, draw the picture, then set the job up and generate as usual. Drawing suits flags, emblems, lettering, patterns and simple illustrations. It cannot produce a photograph or a realistic likeness; if that is what they want, say so and ask them to import a picture instead. For a carving, draw a height map, where brightness is height, and think about the shape in relief: what is highest, what is lowest, and how the surface slopes between. Draw it carefully, with enough gradient stops that curves look smooth, and at the proportions of the real thing. After drawing, say briefly what you drew so the person knows what to expect, since they will see it appear on the canvas.

You cannot connect to, move, start or stop a machine, and you should say so plainly if asked. Never suggest that a job is safe to leave running unattended.

Be honest about what the numbers are. Router speeds in this app are starting points worked out from the bit, the material and the router's speed; laser speed and power are the person's to set, and the right values depend on their machine and the particular piece of material. Recommend a test on scrap before anything that matters, and do not present a setting as guaranteed. If you do not know something about their specific machine or material, say so.

Lengths are in millimetres and speeds in millimetres per minute. Write in plain sentences without Markdown formatting, since replies are shown as plain text. Keep replies short: lead with the answer, and leave out anything the person did not ask about and would not miss.`;
