import type { MachineKind } from "./ipc";
import { css, paintMaterial, type Material } from "./materials";

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
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
  tool: { diameter: number; ballNose: boolean };
}

/** Width of a laser mark in millimetres. */
const LASER_LINE_MM = 0.14;
const MAX_CANVAS_PX = 2048;
const MAX_PX_PER_MM = 8;
/** Cells along the longer side of the height grid used for router jobs. */
const GRID_CELLS = 512;

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
  private readonly stamp: { dx: number; dy: number; lift: number }[] = [];
  private readonly deepest: number;
  /** Segments already drawn or cut. */
  private applied = 0;

  constructor(setup: SimulationSetup) {
    this.setup = setup;
    const { moves, stock, material, kind, tool } = setup;
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

      // The bit's footprint in grid cells, with how far above the tip each part is.
      const radius = tool.diameter / 2;
      const reach = Math.ceil(radius * this.pxPerMm);
      for (let dy = -reach; dy <= reach; dy++) {
        for (let dx = -reach; dx <= reach; dx++) {
          const r = Math.hypot(dx, dy) / this.pxPerMm;
          if (r > radius) continue;
          const lift = tool.ballNose ? radius - Math.sqrt(radius * radius - r * r) : 0;
          this.stamp.push({ dx, dy, lift });
        }
      }
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
    const { moves, stock, material } = this.setup;
    const ctx = this.canvas.getContext("2d")!;
    const scale = this.pxPerMm;
    ctx.lineCap = "round";
    ctx.lineJoin = "round";
    ctx.lineWidth = Math.max(1, LASER_LINE_MM * scale);
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
  private carve(from: number, to: number) {
    const { moves, stock } = this.setup;
    const heights = this.heights!;
    const scale = this.pxPerMm;
    const [w, h] = [this.gridWidth, this.gridHeight];

    for (let i = from; i < to; i++) {
      const a = i * 4;
      const b = a + 4;
      if (moves[b + 3] < 0) continue;
      const [ax, ay, az] = [(moves[a] - stock.x) * scale, (moves[a + 1] - stock.y) * scale, moves[a + 2]];
      const [bx, by, bz] = [(moves[b] - stock.x) * scale, (moves[b + 1] - stock.y) * scale, moves[b + 2]];
      if (az >= 0 && bz >= 0) continue;
      // Half a cell per step, so the bit's path leaves no gaps.
      const steps = Math.max(1, Math.ceil(Math.hypot(bx - ax, by - ay) * 2));
      for (let s = 0; s <= steps; s++) {
        const t = s / steps;
        const cx = Math.round(ax + (bx - ax) * t);
        const cy = Math.round(ay + (by - ay) * t);
        const z = az + (bz - az) * t;
        for (const { dx, dy, lift } of this.stamp) {
          const x = cx + dx;
          const y = cy + dy;
          if (x < 0 || y < 0 || x >= w || y >= h) continue;
          const index = y * w + x;
          if (z + lift < heights[index]) heights[index] = z + lift;
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
    const slope = this.pxPerMm * 0.9;
    const { thickness } = this.setup;

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
        const left = heights[index - (x > 0 ? 1 : 0)];
        const up = heights[index - (y > 0 ? w : 0)];
        // Surfaces that face the light (rising towards the lower right) are brighter.
        const light = Math.min(1.3, Math.max(0.5, 1 + ((z - left) + (z - up)) * slope * 0.35));
        const shade = light * (1 - 0.3 * Math.min(1, -z * depthScale));
        out[o] = base[o] * shade;
        out[o + 1] = base[o + 1] * shade;
        out[o + 2] = base[o + 2] * shade;
      }
    }
    ctx.putImageData(image, 0, 0);
  }
}
