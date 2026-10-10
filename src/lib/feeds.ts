import type { Material } from "./materials";
import type { Tool } from "./tools";

export interface CutSettings {
  /** Cutting speed in mm/min. */
  feed: number;
  plungeFeed: number;
  depthPerPass: number;
  /** Distance between rows when carving a surface. */
  stepover: number;
}

/** The size the material figures are given for: a 1/8" bit. */
const REFERENCE_DIAMETER = 3.175;

/**
 * Starting speeds for a bit in a material on a light, belt-driven hobby router.
 *
 * Feed is spindle speed × flutes × chip load, where chip load is the thickness
 * each cutting edge removes per turn. The chip loads used are those hobby
 * machines run at, far below what a rigid industrial machine could take, and
 * they scale with the bit's diameter. Returns null for materials that are not
 * routed.
 *
 * These are starting points to test on scrap, not guarantees: the right
 * numbers depend on the actual bit, the wood and the machine's condition.
 */
export function cutSettings(tool: Tool, material: Material, spindleRpm: number): CutSettings | null {
  const { machining } = material;
  if (!machining) return null;

  // A tapered bit is far stiffer than a straight bit as thin as its tip, so it
  // is treated as a bit midway between its tip and its shank, up to 1/8".
  const tapered = tool.kind === "taperedBall";
  const working = tapered ? Math.min(REFERENCE_DIAMETER, (tool.diameter + tool.shank) / 2) : tool.diameter;
  const size = Math.min(2, Math.max(0.4, working / REFERENCE_DIAMETER));
  // A V-bit or a tapered bit cuts with a fragile tip, so it is run more gently.
  const chipLoad = machining.chipLoad * size * (tool.kind === "vbit" ? 0.7 : tapered ? 0.8 : 1);
  const feed = Math.min(3000, Math.max(100, Math.round((spindleRpm * tool.flutes * chipLoad) / 10) * 10));
  const depth = Math.min(tool.cutLength, working * machining.depthFactor);

  return {
    feed,
    plungeFeed: Math.max(50, Math.round((feed * 0.3) / 10) * 10),
    depthPerPass: Math.max(0.05, Math.round(depth * 100) / 100),
    // A ball nose needs close rows for a smooth surface; a flat end can step further.
    stepover:
      Math.round(tool.diameter * (tool.kind === "ball" ? 0.12 : tapered ? 0.25 : 0.4) * 100) / 100,
  };
}
