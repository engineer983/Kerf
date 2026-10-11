import type { MachineKind, Sheet, TextSpec } from "./ipc";

// Designs are made from measurements rather than traced from a picture: a
// box with finger joints, or a picture sliced into layers to stack. They are
// exact lines, kept at the size they were drawn.

/** The colours a design draws its lines in, as the backend names them. */
export const CUT_COLOR = "#000000";
export const MARK_COLOR = "#1e6fd9";

/** What the person asked for when making a box. */
export interface BoxInput {
  /** Outside size, in millimetres. */
  width: number;
  depth: number;
  height: number;
  thickness: number;
  /** Zero lets the app choose. */
  fingerWidth: number;
  lid: boolean;
  /** Laser: the width of the beam's cut. */
  kerf: number;
  /** Router: the bit the panels will be cut with. */
  toolId: string | null;
}

/** What the person asked for when slicing a picture into layers. */
export interface LayersInput {
  layers: number;
  widthMm: number;
  smallestPieceMm: number;
  marks: boolean;
  source: "brightness" | "depth";
  lightIsHigh: boolean;
}

interface Made {
  sheets: Sheet[];
  /** Which sheet is on the canvas, counting from 0. */
  sheet: number;
  /** The kind of machine the lines were drawn for. */
  madeFor: MachineKind;
}

export type Design =
  | (Made & { kind: "box"; input: BoxInput })
  | (Made & { kind: "layers"; input: LayersInput })
  /** Words set in a font. */
  | (Made & { kind: "text"; input: TextSpec })
  /** A drawing opened from a file. */
  | (Made & { kind: "drawing"; input: { name: string } });

/**
 * Whether a design must stay the size it was drawn at. A box's joints and a
 * stack's layers only fit at that size; words and drawings can be any size.
 */
export const isFixed = (design: Design) => design.kind === "box" || design.kind === "layers";

export const DEFAULT_TEXT: TextSpec = { text: "", font: "", sizeMm: 20, bold: false, italic: false, align: "center" };

export const DEFAULT_BOX: BoxInput = { width: 150, depth: 100, height: 80, thickness: 3, fingerWidth: 0, lid: false, kerf: 0.15, toolId: null };
export const DEFAULT_LAYERS: LayersInput = { layers: 5, widthMm: 150, smallestPieceMm: 4, marks: true, source: "brightness", lightIsHigh: true };

/** The space inside a box, in millimetres. */
export function insideOf(box: BoxInput): [number, number, number] {
  const t = box.thickness;
  return [box.width - 2 * t, box.depth - 2 * t, box.height - (box.lid ? 2 : 1) * t];
}

const tidy = (mm: number) => Number(mm.toFixed(1));

/** A design in a sentence, for the panel and for the assistant. */
export function describeDesign(design: Design): string {
  const pieces = design.sheets.reduce((count, sheet) => count + sheet.parts.length, 0);
  const sheets = design.sheets.length === 1 ? "one sheet" : `${design.sheets.length} sheets`;
  if (design.kind === "box") {
    const { width, depth, height, thickness, lid } = design.input;
    const [w, d, h] = insideOf(design.input).map(tidy);
    return `${lid ? "A closed" : "An open-topped"} box ${tidy(width)} x ${tidy(depth)} x ${tidy(height)} mm outside (${w} x ${d} x ${h} mm inside) in ${tidy(thickness)} mm material: ${pieces} panels on ${sheets}.`;
  }
  if (design.kind === "text" || design.kind === "drawing") {
    const { widthMm, heightMm } = design.sheets[0];
    const size = `drawn ${tidy(widthMm)} x ${tidy(heightMm)} mm, and can be resized`;
    return design.kind === "text"
      ? `The words "${design.input.text.replace(/\s+/g, " ").trim()}" in ${design.input.font}, as the outlines of their letters, ${size}.`
      : `The drawing "${design.input.name}", as exact lines, ${size}.`;
  }
  return `The picture as ${pieces} layers to stack, each ${tidy(design.input.widthMm)} mm wide, on ${sheets}. Layer 1 is the base; glue each layer onto the one before.`;
}

/** Longest side, in pixels, of the picture made of a design for the gallery. */
const PICTURE_PX = 1200;

/** A design drawn as a picture, so that the project has something to show in the gallery. */
export async function pictureOf(sheet: Sheet, name: string): Promise<File> {
  // Hairlines would vanish at this size, so they are drawn heavier.
  const svg = sheet.svg.replace(/stroke-width="[^"]*"/, `stroke-width="${(Math.max(sheet.widthMm, sheet.heightMm) / 300).toFixed(2)}"`);
  const url = URL.createObjectURL(new Blob([svg], { type: "image/svg+xml" }));
  try {
    const drawing = new Image();
    drawing.src = url;
    await drawing.decode();
    const scale = PICTURE_PX / Math.max(sheet.widthMm, sheet.heightMm);
    const canvas = document.createElement("canvas");
    canvas.width = Math.max(1, Math.round(sheet.widthMm * scale));
    canvas.height = Math.max(1, Math.round(sheet.heightMm * scale));
    const ctx = canvas.getContext("2d")!;
    ctx.fillStyle = "#fff";
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    ctx.drawImage(drawing, 0, 0, canvas.width, canvas.height);
    const png = await new Promise<Blob | null>((done) => canvas.toBlob(done, "image/png"));
    if (!png) throw new Error("The design could not be drawn.");
    return new File([png], `${name}.png`, { type: "image/png" });
  } finally {
    URL.revokeObjectURL(url);
  }
}

/**
 * The closed line that words follow round something `width` by `height`,
 * as SVG path data about the origin, running `out` outside it. It mirrors
 * the backend's, for previews: it starts opposite the side the words are on,
 * so that half way along is the middle of that side, and runs so that the
 * words read left to right there.
 */
export function ringPath(shape: "circle" | "oval" | "frame", side: "top" | "bottom", width: number, height: number, out: number): string {
  const top = side === "top";
  const start = top ? 1 : -1;
  const sweep = top ? 1 : 0;
  if (shape !== "frame") {
    const [rx, ry] =
      shape === "circle"
        ? [Math.hypot(width, height) / 2 + out, Math.hypot(width, height) / 2 + out]
        : [width / Math.SQRT2 + out, height / Math.SQRT2 + out];
    return `M0 ${start * ry}A${rx} ${ry} 0 1 ${sweep} 0 ${-start * ry}A${rx} ${ry} 0 1 ${sweep} 0 ${start * ry}`;
  }
  const [hw, hh] = [width / 2 + out, height / 2 + out];
  const c = Math.min(hw, hh) * 0.35;
  const [near, far] = [-start * hh, start * hh];
  const toward = (from: number, by: number) => from - start * by;
  return (
    `M0 ${far}L${-hw + c} ${far}A${c} ${c} 0 0 ${sweep} ${-hw} ${toward(far, c)}` +
    `L${-hw} ${toward(near, -c)}A${c} ${c} 0 0 ${sweep} ${-hw + c} ${near}` +
    `L${hw - c} ${near}A${c} ${c} 0 0 ${sweep} ${hw} ${toward(near, -c)}` +
    `L${hw} ${toward(far, c)}A${c} ${c} 0 0 ${sweep} ${hw - c} ${far}Z`
  );
}
