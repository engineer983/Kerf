import { DEFAULT_BOX, DEFAULT_LAYERS, DEFAULT_TEXT, describeDesign } from "./design";
import * as ipc from "./ipc";
import type { OperationKind } from "./ipc";
import { findMaterial, MATERIALS } from "./materials";
import { creditLine, loadAsNewProject, loadFoundPicture, pictureFromAddress, pictureName, toPngFile } from "./pictures";
import { TOOL_KIND_LABELS } from "./tools";
import { artworkRect, stockRect, useProject } from "../state/project";
import { pictureEndpoint, usePictures } from "../state/pictures";
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
  /** Pictures a search found, shown to the person as thumbnails. */
  pictures?: ipc.FoundPicture[];
}

/** What the person is told is happening while a slow tool runs. */
export function toolActivity(name: string): string {
  switch (name) {
    case "generate_picture":
      return "Making the picture… this can take a minute or two";
    case "find_pictures":
      return "Searching for pictures…";
    case "make_text":
      return "Drawing the words…";
    case "use_found_picture":
    case "use_picture_from_address":
      return "Downloading the picture…";
    default:
      return "Working on the project…";
  }
}

const OPERATION_TYPES = ["laserImage", "laserFill", "laserCut", "laserCutout", "cncProfile", "cncRelief", "cncCutout"] as const;

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
        sharpen: number("How strongly edges and fine detail are brought out, 0 (off) to 2. About 0.8 makes an engraved photograph look crisper and more lifelike; leave it at 0 for a carving."),
        threshold: { description: 'Dark/light dividing level for outlines: "auto", or 0 to 255.', anyOf: [{ type: "string", enum: ["auto"] }, { type: "number" }] },
        invert: { type: "boolean", description: "Trace the light areas instead of the dark ones." },
        ignoreSpecks: number("Drop shapes smaller than this many pixels across, 0 to 16."),
        smoothCurves: { type: "boolean" },
      },
    },
  },
  {
    name: "optimize_picture",
    description:
      "Tune the picture's adjustments to what the job makes from it, by measuring how light and dark the picture is. For shaded engraving and for carving it sets brightness and contrast so the tones fill the whole range, and smooths a carving slightly; for traced shapes it sets an automatic threshold, a little smoothing, speck removal, and which of light or dark is traced. Set the operations up first, since they decide what it tunes for. Returns what it changed. Use it as a starting point when a picture looks flat, faint or noisy, then adjust with update_trace if the person wants something different.",
    schema: { type: "object", properties: {} },
  },
  {
    name: "remove_background",
    description:
      "Find the subject of the loaded picture and remove everything behind it, leaving white, which a laser leaves alone and which is not traced. A model on this computer does it; it is best on photographs of people and of things that stand clear of what is behind them, and it does poorly on logos and drawings, which do not need it. It can lose part of a subject that is the same shade as the background. Afterwards tell the person they can correct the result by hand with Remove background… in the Design step, by drawing round what to keep or remove, and that Restore the picture brings the background back. Any estimated depth is dropped, so estimate it again for a carving.",
    schema: { type: "object", properties: {} },
  },
  {
    name: "set_material",
    description:
      "Change the material, the size of the piece, and where it sits on the bed. Changing the material recalculates router speeds. Moving the piece carries the design with it, so it stays where it was on the piece. This only changes the layout on screen.",
    schema: {
      type: "object",
      properties: {
        material: { type: "string", enum: MATERIALS.map((m) => m.id) },
        thickness: number("Millimetres."),
        width: number("Left to right, millimetres."),
        length: number("Front to back, millimetres."),
        leftMm: number("Distance of the piece's left edge from the bed's left edge, millimetres. 0 is the origin corner."),
        frontMm: number("Distance of the piece from the bed edge on the origin side (front for a front-left origin, back for a back-left one), millimetres."),
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
        offset: number("Cutout (router or laser): distance of the cut line outside the artwork, mm; negative is inside."),
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
      "Start a new, empty project. The current project is saved first, so nothing is lost. Afterwards there is no picture: either the person imports one, or you get one with create_picture, generate_picture or find_pictures.",
    schema: { type: "object", properties: {} },
  },
  {
    name: "create_picture",
    description:
      "Draw a picture yourself, as SVG, and load it as the project's picture. Use this when the person asks you to make or draw something (a flag, an emblem, lettering, a simple illustration or pattern) rather than supplying a picture. It replaces the current picture and starts a new project. You can only draw what SVG can express; for anything realistic use generate_picture or find_pictures instead. For a 3D carving, draw a height map and set heightMap to true: white is the highest point and black the lowest, and smooth gradients between them become smooth slopes, so rolling waves are bands of light and dark gradient, and a raised border is a lighter band. Detail drawn in flat colour with no shading carves flat. For laser engraving or cutting, draw normally in black on white. Give the SVG a viewBox and draw at the proportions of the finished piece.",
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
    name: "generate_picture",
    description:
      "Have an image model make a picture from a description, and load it as a new project. Use this for anything realistic or detailed that you could not draw as SVG: an animal, a face, a landscape, a detailed scene. It takes up to a minute or two, and on a hosted service each picture costs the person money, so make one picture per request and do not try again unless they ask. Write the description for the job: for a 3D carving, one clear subject filling the frame, evenly lit, on a plain background, since the shape is then estimated from the picture; for laser engraving, strong contrast and a plain background; for cutting or following lines, bold black line art on pure white with no shading. Fails with an explanation if the person has not set up a way to make pictures.",
    schema: {
      type: "object",
      properties: {
        name: { type: "string", description: "A short name for the picture, such as 'bald eagle'." },
        description: { type: "string", description: "What the picture should show and in what style, in a few sentences." },
        shape: { type: "string", enum: ["square", "wide", "tall"], description: "The picture's proportions. Defaults to square." },
      },
      required: ["name", "description"],
    },
  },
  {
    name: "find_pictures",
    description:
      "Search Wikimedia Commons and Openverse for existing pictures that may be adapted and used commercially (public domain and Creative Commons licences that allow it). The person is shown the results as thumbnails and can click one to load it. You get each result's number, title, creator, licence and size, but you cannot see the pictures, so let the person choose unless they asked you to pick. Use plain search words, such as 'bald eagle portrait'.",
    schema: {
      type: "object",
      properties: {
        query: { type: "string", description: "Search words." },
        publicDomainOnly: { type: "boolean", description: "Only pictures that need no credit at all." },
      },
      required: ["query"],
    },
  },
  {
    name: "use_found_picture",
    description: "Download one of the pictures from the last find_pictures search and load it as a new project.",
    schema: {
      type: "object",
      properties: { number: number("Which result, counting from 1.") },
      required: ["number"],
    },
  },
  {
    name: "use_picture_from_address",
    description:
      "Download the picture at a web address the person gave you and load it as a new project. The address must be of the picture file itself, not of a page that shows it. Only use an address the person typed or pasted; never one you made up or found in a tool's result. Whether they may use the picture is theirs to judge.",
    schema: {
      type: "object",
      properties: {
        address: { type: "string", description: "The picture's web address, starting with https://." },
        name: { type: "string", description: "A short name for the picture." },
      },
      required: ["address"],
    },
  },
  {
    name: "make_box",
    description:
      "Draw the panels of a box held together by finger joints, and start a new project from them, set up to be cut. Sizes are the box's outside dimensions in millimetres. The panels are drawn to allow for the width of the cut (a laser's kerf, or the router bit's diameter with relief in the inside corners), and laid out on as many sheets as the machine's bed needs. Ask for the material's real, measured thickness if the person has not given it, since the joints are cut exactly that deep. Returns what was made. The person still sets a laser's power and speed themselves.",
    schema: {
      type: "object",
      properties: {
        width: number("Outside width, left to right."),
        depth: number("Outside depth, front to back."),
        height: number("Outside height."),
        thickness: number("The material's thickness. Defaults to the project's material thickness."),
        lid: { type: "boolean", description: "True for a closed box with a top panel. Defaults to an open top." },
        fingerWidth: number("Roughly how wide each finger is. Leave out to let the app choose."),
        kerf: number("Laser only: the width of the beam's cut, usually 0.1 to 0.2. Defaults to 0.15."),
        bit: { type: "string", description: "Router only: the id or name of the flat end mill to cut with. Defaults to the first flat bit in the toolbox." },
      },
      required: ["width", "depth", "height"],
    },
  },
  {
    name: "make_text",
    description:
      "Draw words in a font installed on the person's computer, as the outlines of their letters. With addToDesign true they are added to what is already on the material, which is how a sign is made: a picture with words above, below or over it. Otherwise they start a new project on their own. The words can be straight, or bent along a curve to arch over a picture or run under it. Added words are placed in the middle of the material; move and size them with set_added_item. Use this for signs, name plates, labels and lettering; it is exact, where drawing letters yourself in create_picture is not. If the font asked for is not installed, the error lists some that are. Bold, simple fonts suit small sizes.",
    schema: {
      type: "object",
      properties: {
        text: { type: "string", description: "The words. Use a line break for a new line." },
        font: { type: "string", description: "A font family's name, such as Arial. Leave out for a plain default." },
        size: number("The font's size in millimetres; capital letters stand about seven tenths of it. Defaults to 20."),
        bold: { type: "boolean" },
        italic: { type: "boolean" },
        align: { type: "string", enum: ["left", "center", "right"], description: "How lines of different lengths line up. Defaults to center." },
        curve: number("Degrees the words bend round a circle, -350 to 350. 0 is straight; above 0 arches over the top (to go above a picture), below 0 curves underneath (to go below one). 180 is half way round."),
        around: {
          type: "object",
          description: "With addToDesign: set the words round the main artwork instead of straight or on a plain curve. They are drawn at the given size to fit round the artwork as it is now and placed exactly, so do not move or resize them afterwards. Fails if the words are too long to go round at that size.",
          properties: {
            shape: { type: "string", enum: ["circle", "oval", "frame"], description: "The line they follow: the circle through the artwork's corners, an oval that hugs it closer, or its own frame with rounded corners." },
            side: { type: "string", enum: ["top", "bottom"], description: "Centred over the top, or under the bottom." },
            gap: { type: "number", description: "Clear space between the artwork and the words, mm. Defaults to 5." },
          },
          required: ["shape", "side"],
        },
        addToDesign: { type: "boolean", description: "True to add the words to the current design. False, the default, starts a new project from them." },
      },
      required: ["text"],
    },
  },
  {
    name: "set_added_item",
    description:
      "Move, resize or remove one of the text or drawing items added beside the main artwork. Items are numbered from 1 in the order get_project lists them under `added`. Sizes and distances are in millimetres, measured as for set_artwork; an item keeps its proportions, so give a width or a height, not both. To change an item's words, remove it and add it again.",
    schema: {
      type: "object",
      properties: {
        item: number("Which added item, counting from 1."),
        width: number("Its width."),
        height: number("Its height."),
        fromLeft: number("Distance of its left edge from the left of the bed."),
        fromEdge: number("Distance from the bed edge on the machine's origin side."),
        remove: { type: "boolean", description: "True to take it off the material." },
      },
      required: ["item"],
    },
  },
  {
    name: "make_layers",
    description:
      "Slice the project's picture into layers to cut from thin sheet and glue into a stack, like the contour lines of a map: layer 1 is a whole base and each layer after it is the part of the picture standing above the next level. Use it when the person wants layered or stacked art, a topographic look, or a 3D effect from flat sheet. Needs a picture loaded. For a photograph or a drawing of an object, take the heights from estimated depth; for a height map or simple artwork, from brightness. On a laser each layer can be marked with where the next one sits. Returns what was made.",
    schema: {
      type: "object",
      properties: {
        layers: number("How many layers, counting the base: 2 to 12. Defaults to 5."),
        width: number("Width of the finished piece in millimetres."),
        heightsFrom: { type: "string", enum: ["brightness", "depth"], description: "Defaults to depth for a photograph, brightness otherwise." },
        highestParts: { type: "string", enum: ["light", "dark"], description: "Which end stands highest: light (or near), or dark (or far). Defaults to light." },
        smallestPiece: number("Pieces smaller than this across, in millimetres, are left out. Defaults to 4."),
        marks: { type: "boolean", description: "Laser only: mark each layer with the outline of the next. Defaults to true." },
      },
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

/** The most looser wordings one request for pictures may try. */
const MAX_LOOSER_SEARCHES = 5;
const FILLER_WORDS = new Set(["a", "an", "the", "of", "on", "in", "at", "with", "and", "or", "for", "to", "over", "across", "image", "images", "picture", "pictures", "photo", "photos"]);

/**
 * The searches to try for a request. `exact` is the words as given, then
 * without filler words, to be tried in that order. `looser` leaves out each
 * remaining word in turn, for when the exact wordings find nothing.
 */
export function searchAttempts(query: string): { exact: string[]; looser: string[] } {
  const given = query.trim().replace(/\s+/g, " ").slice(0, 200);
  const words = given.split(" ").filter((word) => !FILLER_WORDS.has(word.toLowerCase()));
  const exact = [...new Set([given, words.join(" ")].filter(Boolean))];
  const looser = words.length > 2 ? words.map((_, skip) => words.filter((_, i) => i !== skip).join(" ")) : [];
  return { exact, looser: [...new Set(looser)].filter((attempt) => !exact.includes(attempt)).slice(0, MAX_LOOSER_SEARCHES) };
}

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
    // A box or a set of layers, which is cut in place of the traced picture
    // and cannot be resized.
    design: s.design && { summary: describeDesign(s.design), sheets: s.design.sheets.length, sheetShown: s.design.sheet + 1, partsOnThisSheet: s.design.sheets[s.design.sheet].parts },
    material: { id: s.materialId, name: findMaterial(s.materialId).label, thicknessMm: s.thickness, widthMm: s.stockWidth, lengthMm: s.stockLength, leftMm: s.stockLeft, frontMm: s.stockFront },
    // Text and drawings placed on the material beside the main artwork, each
    // movable and resizable on its own. One operation, marked forAddedItems,
    // makes them all.
    added: s.items.map((item, i) => ({
      item: i + 1,
      kind: item.text ? "text" : "drawing",
      name: item.name,
      words: item.text?.text,
      widthMm: item.widthMm,
      heightMm: item.heightMm,
      fromLeftMm: item.left,
      fromEdgeMm: item.front,
    })),
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
      sharpen: preprocess.sharpen ?? 0,
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
      forAddedItems: op.forItems === true || undefined,
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
    const sharpen = num(input, "sharpen", 0, 2);
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
      ...(sharpen !== undefined && { sharpen }),
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

  optimize_picture: async () => {
    const note = await useProject.getState().optimizePicture();
    return { content: note, summary: "Optimised the picture" };
  },

  remove_background: async () => {
    const s = useProject.getState();
    if (!s.image) throw new ToolError("There is no picture loaded.");
    if (s.design) throw new ToolError("This project is a drawing, a box, layers or text, which has no background to remove.");
    await s.removeBackground();
    return {
      content:
        "The background was removed and is now white. The picture as it was can be brought back with Restore the picture in the Design step. If part of the subject is missing, or some background is left, the person can correct it with Remove background… in the Design step.",
      summary: "Removed the background",
    };
  },

  set_material: (input) => {
    const s = useProject.getState();
    const material = choice(input, "material", MATERIALS.map((m) => m.id));
    const thickness = num(input, "thickness", 0.1, 200);
    const width = num(input, "width", 1, 5000);
    const length = num(input, "length", 1, 5000);
    const leftMm = num(input, "leftMm", -5000, 5000);
    const frontMm = num(input, "frontMm", -5000, 5000);
    if (material) s.setMaterial(material);
    if (thickness !== undefined) s.setThickness(thickness);
    if (width !== undefined || length !== undefined) {
      s.setStock({ ...(width !== undefined && { stockWidth: width }), ...(length !== undefined && { stockLength: length }) });
    }
    const moved = leftMm !== undefined || frontMm !== undefined;
    if (moved) {
      s.setStockPlace({ ...(leftMm !== undefined && { stockLeft: leftMm }), ...(frontMm !== undefined && { stockFront: frontMm }) });
    }
    return {
      content: `Material updated. Router speeds were recalculated if the material changed.${moved ? " The design moved with the piece." : ""}`,
      summary: material ? `Set the material to ${findMaterial(material).label}` : moved ? "Moved the material" : "Changed the material's size",
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
    if (type === "cncCutout" || type === "laserCutout") s.addCutout();
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
      content: "A new, empty project is open. There is no picture yet: the person can import one, or you can draw, make or find one.",
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

    const image = await loadAsNewProject(new File([png], `${name}.png`, { type: "image/png" }), { drawnHeightMap: heightMap });
    return {
      content: `The picture "${name}" is loaded (${image.width} x ${image.height} pixels) as a new project${heightMap ? ", and a carving will take its height from brightness" : ""}. Look at the project to see how it was set up.`,
      summary: `Drew a picture: ${name}`,
    };
  },

  generate_picture: async (input) => {
    if (typeof input.description !== "string" || !input.description.trim()) throw new ToolError("description is required.");
    const name = pictureName(input.name, "generated picture");
    const shape = choice(input, "shape", ["square", "wide", "tall"] as const) ?? "square";
    const target = pictureEndpoint();
    if ("problem" in target) {
      throw new ToolError(
        `${target.problem} The person can set this up under Pictures in the assistant's settings (the cog at the top of this panel): a hosted image service with their own key, or an image model on their own computer. Until then you can draw with create_picture or search with find_pictures.`,
      );
    }
    const bytes = await ipc.generatePicture(target.endpoint, target.keyId, input.description.trim().slice(0, 4000), shape);
    const image = await loadAsNewProject(await toPngFile(bytes, name));
    return {
      content: `The picture "${name}" was made and is loaded (${image.width} x ${image.height} pixels) as a new project. You cannot see it; the person can. Look at the project to see how it was set up.`,
      summary: `Made a picture: ${name}`,
    };
  },

  find_pictures: async (input) => {
    if (typeof input.query !== "string" || !input.query.trim()) throw new ToolError("query is required.");
    if (!usePictures.getState().webSearch) {
      throw new ToolError("Searching the web for pictures is switched off. The person can switch it on under Pictures in the assistant's settings.");
    }
    const publicDomainOnly = flag(input, "publicDomainOnly") ?? false;
    // The search only returns pictures matching every word, so a long phrase
    // often finds nothing. Looser versions are tried until one does.
    const { exact, looser } = searchAttempts(input.query);
    let words = "";
    let found: ipc.FoundPicture[] = [];
    for (const attempt of exact) {
      words = attempt;
      found = (await ipc.searchPictures(attempt, publicDomainOnly)).pictures;
      if (found.length > 0) break;
    }
    if (found.length === 0) {
      // Which word to leave out is judged by what the collections hold: the
      // wording that matches the most pictures has dropped the odd word out.
      let most = 0;
      for (const attempt of looser) {
        const results = await ipc.searchPictures(attempt, publicDomainOnly);
        if (results.pictures.length > 0 && results.total > most) {
          most = results.total;
          words = attempt;
          found = results.pictures;
        }
      }
    }
    usePictures.getState().setFound(found);
    if (found.length === 0) {
      return {
        content: "The search ran, but no pictures match those words. Search again with two or three plain words for the subject.",
        summary: `Found no pictures for "${input.query.trim()}"`,
      };
    }
    return {
      content: JSON.stringify({
        searchedFor: words,
        note: "The person can see these as thumbnails and click one to load it. You cannot see them.",
        results: found.map((p, i) => ({ number: i + 1, title: p.title, creator: p.creator || null, licence: p.licence, creditRequired: p.needsCredit, pixels: [p.width, p.height] })),
      }),
      summary: `Found ${found.length} picture${found.length === 1 ? "" : "s"} for "${words}"`,
      pictures: found,
    };
  },

  use_found_picture: async (input) => {
    const { found } = usePictures.getState();
    if (found.length === 0) throw new ToolError("There are no search results. Search with find_pictures first.");
    const index = num(input, "number", 1, found.length);
    if (index === undefined || !Number.isInteger(index)) throw new ToolError(`number must be a whole number from 1 to ${found.length}.`);
    const picture = found[index - 1];
    const image = await loadFoundPicture(picture);
    return {
      content: `Loaded ${creditLine(picture)} (${image.width} x ${image.height} pixels) as a new project. ${picture.needsCredit ? `Tell the person the credit to give: ${picture.attribution || creditLine(picture)}` : "No credit is required."} Source: ${picture.page}`,
      summary: `Loaded ${creditLine(picture)}`,
    };
  },

  use_picture_from_address: async (input) => {
    if (typeof input.address !== "string" || !input.address.trim()) throw new ToolError("address is required.");
    const name = typeof input.name === "string" && input.name.trim() ? input.name : undefined;
    const image = await loadAsNewProject(await pictureFromAddress(input.address, name));
    return {
      content: `The picture is loaded (${image.width} x ${image.height} pixels) as a new project called "${image.name}". You cannot see it; the person can.`,
      summary: `Loaded the picture from ${new URL(input.address.trim()).hostname}`,
    };
  },

  make_box: async (input) => {
    const s = useProject.getState();
    const width = num(input, "width", 10, 3000);
    const depth = num(input, "depth", 10, 3000);
    const height = num(input, "height", 10, 3000);
    if (width === undefined || depth === undefined || height === undefined) throw new ToolError("width, depth and height are required.");
    const flat = useToolbox.getState().tools.filter((t) => t.kind === "flat");
    let toolId = flat[0]?.id ?? null;
    if (typeof input.bit === "string") {
      const wanted = input.bit.toLowerCase();
      const bit = flat.find((t) => t.id === input.bit || t.name.toLowerCase() === wanted);
      if (!bit) throw new ToolError(`No flat end mill called "${input.bit}" is in the toolbox.`);
      toolId = bit.id;
    }
    await s.makeBox({
      width,
      depth,
      height,
      thickness: num(input, "thickness", 0.5, 50) ?? s.thickness,
      lid: flag(input, "lid") ?? false,
      fingerWidth: num(input, "fingerWidth", 0, 500) ?? 0,
      kerf: num(input, "kerf", 0, 2) ?? DEFAULT_BOX.kerf,
      toolId,
    });
    const made = useProject.getState().design!;
    return {
      content: `${describeDesign(made)} It is loaded as a new project and set up to be cut${made.sheets.length > 1 ? `, one sheet at a time; sheet 1 is showing, and the person chooses the others under Operations` : ""}.${s.machineKind === "laser" ? " The laser's power and speed for cutting through are the person's to set." : " Nothing holds a panel once it is cut free, so the sheet must be stuck down."}`,
      summary: "Made a box",
    };
  },

  make_text: async (input) => {
    if (typeof input.text !== "string" || !input.text.trim()) throw new ToolError("text is required.");
    const fonts = await ipc.listFonts();
    const usual = ["Arial", "Segoe UI", "Helvetica", "DejaVu Sans", "Liberation Sans"];
    let font = usual.find((name) => fonts.includes(name)) ?? fonts[0] ?? "";
    if (typeof input.font === "string" && input.font.trim()) {
      const wanted = input.font.trim().toLowerCase();
      const found = fonts.find((name) => name.toLowerCase() === wanted);
      if (!found) {
        const near = fonts.filter((name) => name.toLowerCase().includes(wanted.split(" ")[0]));
        throw new ToolError(`The font "${input.font}" is not installed. ${near.length ? `Similar: ${near.slice(0, 12).join(", ")}.` : `Some that are: ${fonts.slice(0, 30).join(", ")}.`}`);
      }
      font = found;
    }
    const spec = {
      text: input.text.slice(0, 2000),
      font,
      sizeMm: num(input, "size", 1, 1000) ?? DEFAULT_TEXT.sizeMm,
      bold: flag(input, "bold") ?? false,
      italic: flag(input, "italic") ?? false,
      align: choice(input, "align", ["left", "center", "right"] as const) ?? "center",
      bend: num(input, "curve", -350, 350) ?? 0,
      around: null as ipc.TextSpec["around"],
    };
    if (typeof input.around === "object" && input.around !== null) {
      const ring = input.around as Input;
      const shape = choice(ring, "shape", ["circle", "oval", "frame"] as const);
      const side = choice(ring, "side", ["top", "bottom"] as const);
      if (!shape || !side) throw new ToolError("around needs a shape and a side.");
      spec.around = { shape, side, gap: num(ring, "gap", -100, 500) ?? 5 };
      spec.bend = 0;
    }
    const project = useProject.getState();
    if (flag(input, "addToDesign")) {
      if (!project.image) throw new ToolError("There is nothing on the material to add to yet. Leave addToDesign out to start a project from the words.");
      await project.addText(spec);
      const items = useProject.getState().items;
      const added = items[items.length - 1];
      return {
        content: spec.around
          ? `Added as item ${items.length}, set round the artwork: ${added.widthMm} x ${added.heightMm} mm, ${added.left} mm from the left and ${added.front} mm from the edge. It is already in place; check that everything still fits on the material.`
          : `Added as item ${items.length}: ${added.widthMm} x ${added.heightMm} mm, in the middle of the material (${added.left} mm from the left, ${added.front} mm from the edge). Move or resize it with set_added_item so that it sits where it should and does not overlap anything it should not.`,
        summary: `Added the text "${added.name}"`,
      };
    }
    await project.makeText(spec);
    const made = useProject.getState().design!;
    return { content: `${describeDesign(made)} It is loaded as a new project. Look at the project to see how it was set up.`, summary: "Drew the text" };
  },

  set_added_item: (input) => {
    const s = useProject.getState();
    const index = num(input, "item", 1, Math.max(1, s.items.length));
    const item = index !== undefined && Number.isInteger(index) ? s.items[index - 1] : undefined;
    if (!item) throw new ToolError(s.items.length ? `item must be a whole number from 1 to ${s.items.length}.` : "Nothing has been added to the design.");
    if (flag(input, "remove")) {
      s.removeItem(item.id);
      return { content: "Removed. The remaining items are renumbered.", summary: `Removed "${item.name}"` };
    }
    const width = num(input, "width", 1, 5000);
    const height = num(input, "height", 1, 5000);
    const left = num(input, "fromLeft", -5000, 5000);
    const front = num(input, "fromEdge", -5000, 5000);
    if (width !== undefined) s.updateItem(item.id, { widthMm: width });
    else if (height !== undefined) s.updateItem(item.id, { heightMm: height });
    if (left !== undefined || front !== undefined) {
      s.updateItem(item.id, { ...(left !== undefined && { left }), ...(front !== undefined && { front }) });
    }
    const now = useProject.getState().items.find((i) => i.id === item.id)!;
    return {
      content: `Item ${index} is now ${now.widthMm} x ${now.heightMm} mm, ${now.left} mm from the left and ${now.front} mm from the edge.`,
      summary: `Moved or resized "${item.name}"`,
    };
  },

  make_layers: async (input) => {
    const s = useProject.getState();
    if (!s.image) throw new ToolError("There is no picture loaded yet.");
    const layers = num(input, "layers", 2, 12);
    const source = choice(input, "heightsFrom", ["brightness", "depth"] as const) ?? (s.image.photo ? "depth" : "brightness");
    await s.makeLayers({
      layers: layers === undefined ? DEFAULT_LAYERS.layers : Math.round(layers),
      widthMm: num(input, "width", 10, 5000) ?? (s.design ? DEFAULT_LAYERS.widthMm : s.widthMm),
      smallestPieceMm: num(input, "smallestPiece", 0, 100) ?? DEFAULT_LAYERS.smallestPieceMm,
      marks: flag(input, "marks") ?? true,
      source,
      lightIsHigh: (choice(input, "highestParts", ["light", "dark"] as const) ?? "light") === "light",
    });
    const made = useProject.getState().design!;
    return {
      content: `${describeDesign(made)} The job is set up to cut them${made.sheets.length > 1 ? `, one sheet at a time; sheet 1 is showing` : ""}. Fewer layers came out than asked for if nothing in the picture stands high enough for the upper ones.`,
      summary: "Sliced the picture into layers",
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

How the app works, in numbered steps: 1 Device, the machine; 2 Material, what it is and how big the piece is; 3 Design, what goes on the material; 4 Trace, how a picture is turned into outlines; 5 Operations, what the machine does; 6 Machine, running it. The design starts with one main thing (a picture, a drawing, words or a box) and can have more words and drawings added around it, each placed and sized on its own, which is how a sign is laid out. Everything is fitted to the material, so the material's size should be right before the design is arranged. One operation makes all the added items. Whatever order operations are listed in, the job marks and engraves first, the added items next, then anything that cuts right through (the main artwork's laser lines, router lines as deep as the material, and a box's or layers' parts), and any cutout last, so nothing is cut free before it has been marked. A cutout goes round everything together. A laser can engrave the picture with its shading (image), fill traced shapes, or follow their outlines. A router can follow traced outlines (profile), carve the picture as a 3D relief, or cut the piece out with holding tabs. A relief takes its heights either from the picture's brightness or from an estimate of depth, which suits photographs and drawings of objects. Generating produces G-code and a preview; nothing reaches a machine until the person sends it themselves.

You can read the project and change its settings with your tools. Look at the project before advising or changing anything, since it may have changed since you last looked. When the person asks for a result ("make it deeper", "it's taking too long"), make the change rather than describing how they could, then say in a sentence or two what you changed and why. Your changes can be undone with Ctrl+Z, so prefer acting to asking, but do ask when the request could reasonably mean quite different things. After changing something that affects the job, generate the G-code again if they will want to see the result, and report the new time.

Most requests are a wish, not a setting: "make it as realistic as possible", "I want this on a cutting board", "it takes too long". Your job is to turn the wish into a complete, sensible setup, setting every relevant thing yourself in one go rather than adjusting one number and leaving the rest. Work out what result they are after, look at the project, make all the changes, generate, and then report: what you set up and why in a few sentences, how long the job will take, and the one or two trade-offs worth knowing. If the job comes out very long, say so and offer the faster alternative you would choose. Ask a question first only when something essential is unknown and guessing wrong would waste material, such as the material's real thickness when a cut-through depends on it. Otherwise state your assumption and proceed.

What you know about getting good results in this app:

Realism and fine detail on a router come from a 3D relief. For a photograph or a drawing of an object, take the height from estimated depth, since brightness is not shape. The depth estimate gives a smooth form, so add a little of the picture's own detail back. A small amount goes a long way: start around 0.1 to 0.2 for line art, logos and lettering, and 0.05 to 0.15 for photographs of people, where more turns skin rough. Raise it only if the person asks for crisper linework, and look at the result rather than assuming more is better. Flatten the background by 0.2 to 0.4 when the subject sits on a backdrop, so the subject uses the full depth. Detail is limited by the bit's tip: choose the smallest ball nose or tapered ball nose in the toolbox that the person will wait for, and keep the stepover to about a quarter of a tapered bit's tip diameter, or a tenth to an eighth of a plain ball nose's diameter. More depth reads as more realism, typically 3 to 6 mm, but leave a few millimetres of material under the deepest cut and stay within the bit's cutting length. A larger piece also shows more detail for the same bit.

Realism on a laser comes from image engraving, which keeps shading; fill and line operations are flat and suit logos and text. Dots (dithering) suit wood and other materials that do not darken evenly with power. A line interval near 0.1 mm is about as fine as a diode laser's spot allows; coarser is faster and lighter. A little extra contrast usually helps a photograph. You do not know the right power and speed for their laser and material, so say so and suggest a small test grid on scrap.

Speed is the opposite trade: a larger stepover or line interval, a larger bit, less depth, or a smaller piece. Say what detail is given up.

Clean signs and logos: on a laser, fill the traced shapes; on a router, follow the traced paths, or carve a relief from brightness with some blur for a smooth, embossed look.

To free the finished piece on a laser, add a laserCutout last: a line around the artwork cut right through, in the same job as the engraving, since a laser has no bit to change. Its power, speed and passes are separate from the engraving's, and what gets through depends on their laser and material, so say they must test it; several faster passes often cut cleaner than one slow one. To free the finished piece on a router, add a cutout last: outside the line, depth equal to the material's thickness, with tabs so the piece cannot come loose. Operations that use different bits cannot run as one job, because the bit must be changed and the height set again in between; include one at a time and tell the person the order to run them in.

Before you finish, check that the artwork fits on the material, that no depth exceeds the bit's cutting length, and that a relief does not go through the material.

You can start a new project, and when the person asks you to create something rather than giving you a picture, you have three ways to get one. Choose by what they want, get the picture, then set the job up and generate as usual.

Draw it yourself as SVG when it is made of shapes: flags, emblems, lettering, patterns, simple illustrations. This is free and immediate. For a carving, draw a height map, where brightness is height, and think about the shape in relief: what is highest, what is lowest, and how the surface slopes between. Draw it carefully, with enough gradient stops that curves look smooth, and at the proportions of the real thing.

Have an image model make it when it should look realistic or richly detailed: animals, people, scenery. This only works if the person has set up a way to make pictures, and on a hosted service each picture costs them money, so make one and let them judge it rather than making several.

Search the web when they want a real photograph or an existing artwork, or when making pictures is not set up. Results are limited to pictures that may be adapted and used commercially. The person sees thumbnails and picks one; you cannot see them. When a picture needs credit, pass on the credit line you are given, and mention that they should check the source page if they plan to sell the piece. The search covers two collections of freely usable pictures, which is far less than a web search engine sees, and it matches words literally, so use two or three plain words for the subject. If nothing suitable turns up, say so plainly and tell them the quick way round it: find a picture in their web browser, right-click it and choose Copy image, then press Ctrl+V in this app. They can also paste a picture's web address to you. Pictures found that way are often someone's copyright, which matters if they plan to sell what they make.

You cannot see a picture that was made or found unless the person has let you see the project's picture, so do not describe it as if you had. Say what you asked for, and ask them how it looks.

You can also make things that need no tracing. Words are drawn in any installed font as the outlines of their letters, ready to engrave, outline or follow with a bit; unlike boxes and layers they can be resized afterwards. The person can also open drawings themselves (SVG files, and PDF, AI and EPS where Inkscape is installed), which are likewise used as exact lines. Two more things need no tracing. A box with finger joints is made from its outside size and the material's thickness: it starts a new project of panels to cut. Stacked layers slice the loaded picture into pieces to cut from thin sheet and glue into a stack. Both are exact lines kept at the size they were drawn, so they cannot be resized or engraved as pictures; to change one, make it again. When one needs more than one sheet, the person cuts a sheet at a time.

The app has a Help guide, opened from the title bar, with good practice for the laser and the router, step-by-step instructions for common jobs, and what to do when results are poor. Point the person to it when they ask how to do something by hand or about safety, as well as answering.

You cannot connect to, move, start or stop a machine, and you should say so plainly if asked. Never suggest that a job is safe to leave running unattended.

Be honest about what the numbers are. Router speeds in this app are starting points worked out from the bit, the material and the router's speed; laser speed and power are the person's to set, and the right values depend on their machine and the particular piece of material. Recommend a test on scrap before anything that matters, and do not present a setting as guaranteed. If you do not know something about their specific machine or material, say so.

Lengths are in millimetres and speeds in millimetres per minute. Write in plain sentences without Markdown formatting, since replies are shown as plain text. Keep replies short: lead with the answer, and leave out anything the person did not ask about and would not miss.`;
