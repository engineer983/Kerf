import type { MachineKind } from "./ipc";
import { css, paintMaterial, type Material } from "./materials";
import type { ToolKind } from "./tools";

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** What the simulation needs to know about a router bit. */
export interface BitShape {
  diameter: number;
  kind: ToolKind;
  /** Included angle of a V-bit, in degrees. */
  angle: number | null;
  /** Side angle of a tapered ball nose, in degrees. */
  taperAngle?: number | null;
  shank?: number;
}

export interface SimulationSetup {
  /** `x, y, z, power` per move, as produced by the G-code generator. */
  moves: number[];
  /** The piece of material, in bed coordinates (mm, Y down). */
  stock: Rect;
  thickness: number;
  material: Material;
  kind: MachineKind;
  /** The router bit; ignored for lasers. */
  tool: BitShape;
  /** The bit of each operation, in order, when they differ. */
  tools?: BitShape[];
  /** Index of the first move of each operation, from the generator. */
  operationStarts?: number[];
  /**
   * Line spacing of an image engraving, in mm. Its rows are then drawn exactly
   * that wide with square ends, so dots neither overlap nor leave gaps and the
   * picture keeps its tones.
   */
  imageInterval?: number;
}

/** Width of a laser mark in millimetres. */
const LASER_LINE_MM = 0.14;
const MAX_CANVAS_PX = 4096;
const MAX_PX_PER_MM = 12;
/** Cells along the longer side of the height grid used for router jobs. */
const GRID_CELLS = 1152;

/**
 * Replays a job onto a piece of material. For a laser it draws the marks; for
 * a router it removes material from a height grid. It can be advanced, or
 * rewound, to any point in the job.
 */
export class Simulation {
  /** What the top of the material looks like at the current point in the job. */
  readonly canvas = document.createElement("canvas");
  /** The bare material, at the same size as `canvas`. */
  readonly base = document.createElement("canvas");
  readonly setup: SimulationSetup;
  readonly segments: number;
  /** Height of the surface per grid cell (0 at the top, negative below); router jobs only. */
  readonly heights: Float32Array | null = null;
  readonly gridWidth: number = 0;
  readonly gridHeight: number = 0;
  /** Increases whenever `canvas` or `heights` change, so views know to refresh. */
  version = 0;
  /** Tool position in bed coordinates, and whether it is cutting, at the current point. */
  head: [number, number, number] = [0, 0, 0];
  cutting = false;

  private readonly pxPerMm: number;
  /** Distance travelled by the end of each move. */
  private readonly travelled: Float64Array;
  private readonly basePixels: Uint8ClampedArray | null = null;
  private readonly deepest: number;
  /** Segments already drawn or cut. */
  private applied = 0;

  constructor(setup: SimulationSetup) {
    this.setup = setup;
    const { moves, stock, material, kind } = setup;
    this.segments = Math.max(0, moves.length / 4 - 1);

    this.travelled = new Float64Array(this.segments + 1);
    let deepest = 0;
    for (let i = 1; i <= this.segments; i++) {
      const a = (i - 1) * 4;
      const b = i * 4;
      const length = Math.hypot(moves[b] - moves[a], moves[b + 1] - moves[a + 1], moves[b + 2] - moves[a + 2]);
      this.travelled[i] = this.travelled[i - 1] + length;
      if (moves[b + 3] >= 0) deepest = Math.min(deepest, moves[b + 2]);
    }
    this.deepest = deepest;

    const longest = Math.max(stock.width, stock.height);
    if (kind === "cnc") {
      this.pxPerMm = GRID_CELLS / longest;
      this.gridWidth = Math.max(2, Math.round(stock.width * this.pxPerMm));
      this.gridHeight = Math.max(2, Math.round(stock.height * this.pxPerMm));
      this.heights = new Float32Array(this.gridWidth * this.gridHeight);

      this.canvas.width = this.base.width = this.gridWidth;
      this.canvas.height = this.base.height = this.gridHeight;
    } else {
      this.pxPerMm = Math.min(MAX_PX_PER_MM, MAX_CANVAS_PX / longest);
      this.canvas.width = this.base.width = Math.max(2, Math.round(stock.width * this.pxPerMm));
      this.canvas.height = this.base.height = Math.max(2, Math.round(stock.height * this.pxPerMm));
    }

    paintMaterial(this.base, material, this.pxPerMm);
    if (kind === "cnc") {
      const ctx = this.base.getContext("2d")!;
      this.basePixels = ctx.getImageData(0, 0, this.base.width, this.base.height).data;
    }
    this.reset();
    this.seek(1);
  }

  private reset() {
    this.applied = 0;
    this.heights?.fill(0);
    const ctx = this.canvas.getContext("2d")!;
    ctx.globalAlpha = 1;
    ctx.drawImage(this.base, 0, 0);
  }

  /** Moves to a point in the job: 0 is the start and 1 the end, by distance travelled. */
  seek(progress: number) {
    const { moves } = this.setup;
    if (this.segments === 0) return;
    const total = this.travelled[this.segments];
    const target = Math.min(1, Math.max(0, progress)) * total;

    // Find the segment that contains the target distance.
    let low = 0;
    let high = this.segments;
    while (low < high) {
      const mid = (low + high) >> 1;
      if (this.travelled[mid + 1] <= target) low = mid + 1;
      else high = mid;
    }
    const complete = Math.min(low, this.segments);

    if (complete < this.applied) this.reset();
    if (complete > this.applied) {
      if (this.setup.kind === "cnc") this.carve(this.applied, complete);
      else this.burn(this.applied, complete);
      this.applied = complete;
      this.version++;
    }

    if (complete >= this.segments) {
      const end = this.segments * 4;
      this.head = [moves[end], moves[end + 1], moves[end + 2]];
      this.cutting = false;
    } else {
      const a = complete * 4;
      const b = a + 4;
      const span = this.travelled[complete + 1] - this.travelled[complete];
      const t = span > 0 ? (target - this.travelled[complete]) / span : 0;
      this.head = [
        moves[a] + (moves[b] - moves[a]) * t,
        moves[a + 1] + (moves[b + 1] - moves[a + 1]) * t,
        moves[a + 2] + (moves[b + 2] - moves[a + 2]) * t,
      ];
      this.cutting = moves[b + 3] >= 0;
    }
  }

  /** Draws the laser marks of segments `from` up to `to`. */
  private burn(from: number, to: number) {
    const { moves, stock, material, imageInterval } = this.setup;
    const ctx = this.canvas.getContext("2d")!;
    const scale = this.pxPerMm;
    ctx.lineCap = imageInterval ? "butt" : "round";
    ctx.lineJoin = "round";
    ctx.lineWidth = imageInterval ? imageInterval * scale : Math.max(1, LASER_LINE_MM * scale);
    ctx.strokeStyle = css(material.engrave);

    // Consecutive marks at one power are drawn as one stroke, so that a
    // path's own joints do not show up as darker spots.
    let power = -1;
    let open = false;
    const flush = () => {
      if (open) ctx.stroke();
      open = false;
    };
    for (let i = from; i < to; i++) {
      const a = i * 4;
      const b = a + 4;
      const p = moves[b + 3];
      if (p <= 0) {
        flush();
        continue;
      }
      if (!open || p !== power) {
        flush();
        power = p;
        ctx.globalAlpha = 0.3 + 0.7 * p;
        ctx.beginPath();
        ctx.moveTo((moves[a] - stock.x) * scale, (moves[a + 1] - stock.y) * scale);
        open = true;
      }
      ctx.lineTo((moves[b] - stock.x) * scale, (moves[b + 1] - stock.y) * scale);
    }
    flush();
    ctx.globalAlpha = 1;
  }

  /** Removes the material swept by the bit over segments `from` up to `to`. */
  /** A bit's reach and profile, for removing material. */
  private shapeOf(tool: BitShape) {
    const tip = tool.diameter / 2;
    const taper = (((tool.taperAngle ?? 0) > 0 ? tool.taperAngle! : 5) * Math.PI) / 180;
    // Where a tapered bit's ball meets its cone, and how high that is.
    const tangent = tip * Math.cos(taper);
    const tangentHeight = tip * (1 - Math.sin(taper));
    // A taper widens all the way to the shank, but only the part within the
    // job's depth can touch the work.
    const radius =
      tool.kind === "taperedBall"
        ? Math.min((tool.shank ?? Infinity) / 2, tangent - this.deepest * Math.tan(taper) + 0.1)
        : tip;
    const slope = tool.kind === "vbit" ? 1 / Math.tan((((tool.angle ?? 90) / 2) * Math.PI) / 180) : 0;
    // How far above its tip the bit's surface is, at a distance from its axis.
    const lift = (r: number) =>
      tool.kind === "ball" || (tool.kind === "taperedBall" && r <= tangent)
        ? tip - Math.sqrt(Math.max(0, tip * tip - r * r))
        : tool.kind === "taperedBall"
          ? tangentHeight + (r - tangent) / Math.tan(taper)
          : tool.kind === "vbit"
            ? r * slope
            : 0;
    return { radius, lift };
  }

  /** Removes the material swept by the bit over segments `from` up to `to`. */
  private carve(from: number, to: number) {
    const { moves, stock, tool, tools, operationStarts } = this.setup;
    const heights = this.heights!;
    const scale = this.pxPerMm;
    const [w, h] = [this.gridWidth, this.gridHeight];
    // Each operation may use its own bit; segments are matched to operations
    // by where each operation's moves begin.
    const shapes = (tools && tools.length > 0 ? tools : [tool]).map((t) => this.shapeOf(t));
    const starts = operationStarts ?? [];
    let operation = 0;

    for (let i = from; i < to; i++) {
      const a = i * 4;
      const b = a + 4;
      if (moves[b + 3] < 0) continue;
      while (operation + 1 < starts.length && i + 1 >= starts[operation + 1]) operation++;
      const { radius, lift } = shapes[Math.min(operation, shapes.length - 1)];
      const reach = radius * scale;
      const [ax, ay, az] = [(moves[a] - stock.x) * scale, (moves[a + 1] - stock.y) * scale, moves[a + 2]];
      const [bx, by, bz] = [(moves[b] - stock.x) * scale, (moves[b + 1] - stock.y) * scale, moves[b + 2]];
      if (az >= 0 && bz >= 0) continue;
      // One cell per step, so the bit's path leaves no gaps.
      const steps = Math.max(1, Math.ceil(Math.hypot(bx - ax, by - ay)));
      for (let s = 0; s <= steps; s++) {
        const t = s / steps;
        // Distances are measured from the bit's true position, not the nearest
        // cell, so rows of a carving do not show up as false ridges.
        const cx = ax + (bx - ax) * t;
        const cy = ay + (by - ay) * t;
        const z = az + (bz - az) * t;
        const x0 = Math.max(0, Math.ceil(cx - reach));
        const x1 = Math.min(w - 1, Math.floor(cx + reach));
        const y0 = Math.max(0, Math.ceil(cy - reach));
        const y1 = Math.min(h - 1, Math.floor(cy + reach));
        // This is the simulation's innermost loop, run hundreds of millions of
        // times on a long job, so it avoids Math.hypot, which is far slower
        // than a multiply and a square root.
        for (let y = y0; y <= y1; y++) {
          const dy2 = (y - cy) * (y - cy);
          for (let x = x0; x <= x1; x++) {
            const r2 = (x - cx) * (x - cx) + dy2;
            if (r2 > reach * reach) continue;
            const bottom = z + lift(Math.sqrt(r2) / scale);
            const index = y * w + x;
            if (bottom < heights[index]) heights[index] = bottom;
          }
        }
      }
    }
    this.shade();
  }
  /** Paints the carved surface: lit from the upper left, darker with depth. */
  private shade() {
    const heights = this.heights!;
    const base = this.basePixels!;
    const [w, h] = [this.gridWidth, this.gridHeight];
    const ctx = this.canvas.getContext("2d")!;
    const image = ctx.createImageData(w, h);
    const out = image.data;
    const depthScale = this.deepest < 0 ? 1 / -this.deepest : 0;
    const perMm = this.pxPerMm;
    const { thickness } = this.setup;
    // Light from the upper left, fairly low, as from a window or a desk lamp.
    const [lx, ly, lz] = [-0.48, -0.56, 0.68];

    for (let y = 0; y < h; y++) {
      for (let x = 0; x < w; x++) {
        const index = y * w + x;
        const z = heights[index];
        const o = index * 4;
        out[o + 3] = 255;
        if (-z >= thickness - 1e-3) {
          // Cut right through: the bed shows.
          out[o] = out[o + 1] = out[o + 2] = 24;
          continue;
        }
        // The surface normal, from the slope across the neighbouring cells.
        const dzdx = ((heights[index + (x < w - 1 ? 1 : 0)] - heights[index - (x > 0 ? 1 : 0)]) * perMm) / 2;
        const dzdy = ((heights[index + (y < h - 1 ? w : 0)] - heights[index - (y > 0 ? w : 0)]) * perMm) / 2;
        const length = Math.sqrt(dzdx * dzdx + dzdy * dzdy + 1);
        // Diffuse light, scaled so that a flat surface keeps the material's own colour.
        const diffuse = Math.max(0, (-dzdx * lx - dzdy * ly + lz) / length) / lz;
        const light = 0.3 + 0.7 * Math.min(1.5, diffuse);
        // Deep areas catch less light from the sky, so they sit a little darker.
        const shade = light * (1 - 0.22 * Math.min(1, -z * depthScale));
        out[o] = base[o] * shade;
        out[o + 1] = base[o + 1] * shade;
        out[o + 2] = base[o + 2] * shade;
      }
    }
    ctx.putImageData(image, 0, 0);
  }
}
