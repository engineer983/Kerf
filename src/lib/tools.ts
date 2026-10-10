export type ToolKind = "flat" | "ball" | "taperedBall" | "vbit";

/** A router bit. Lengths are in millimetres. */
export interface Tool {
  id: string;
  name: string;
  kind: ToolKind;
  /**
   * Cutting diameter. For a V-bit, the diameter at its widest; for a tapered
   * ball nose, the diameter of the ball at the tip (twice its radius).
   */
  diameter: number;
  flutes: number;
  /** Length of the cutting edges: the deepest the bit can cut. */
  cutLength: number;
  shank: number;
  /** Included angle in degrees, for V-bits. */
  angle: number | null;
  /** Angle of each side from the axis in degrees, for tapered ball noses. */
  taperAngle: number | null;
  brand: string;
  /** Where the details came from, if anywhere. */
  url: string;
  notes: string;
}

export const TOOL_KIND_LABELS: Record<ToolKind, string> = {
  flat: "Flat end mill",
  ball: "Ball nose",
  taperedBall: "Tapered ball nose",
  vbit: "V-bit",
};

const GENERIC = "Generic entry with typical dimensions. Check it against your actual bit.";

function generic(id: string, name: string, kind: ToolKind, diameter: number, cutLength: number, shank: number, angle: number | null = null): Tool {
  return { id, name, kind, diameter, flutes: 2, cutLength, shank, angle, taperAngle: null, brand: "", url: "", notes: GENERIC };
}

/**
 * The side angle of a tapered ball nose worked out from its dimensions: the
 * taper runs from the ball at the tip out to the shank over the cutting length.
 */
export function taperFromDimensions(tipDiameter: number, shank: number, cutLength: number): number {
  const rise = Math.max(0, shank / 2 - tipDiameter / 2);
  const degrees = (Math.atan2(rise, Math.max(0.1, cutLength)) * 180) / Math.PI;
  return Math.round(Math.max(0.5, degrees) * 100) / 100;
}

function tapered(id: string, tipRadius: number): Tool {
  // The usual hobby form: a 1/4" shank tapering over about 30 mm to a small ball.
  const [shank, cutLength] = [6.35, 30.5];
  return {
    ...generic(id, `Tapered ball nose R${tipRadius}`, "taperedBall", tipRadius * 2, cutLength, shank),
    taperAngle: taperFromDimensions(tipRadius * 2, shank, cutLength),
  };
}

// Common hobby-router sizes, so the toolbox is useful before anything is added.
export const DEFAULT_TOOLS: Tool[] = [
  generic("flat-1-8", '1/8" flat end mill', "flat", 3.175, 12.7, 3.175),
  generic("flat-1-4", '1/4" flat end mill', "flat", 6.35, 19, 6.35),
  generic("flat-1-16", '1/16" flat end mill', "flat", 1.5875, 6.35, 3.175),
  generic("ball-1-8", '1/8" ball nose', "ball", 3.175, 12.7, 3.175),
  generic("ball-1-16", '1/16" ball nose', "ball", 1.5875, 6.35, 3.175),
  generic("ball-1-4", '1/4" ball nose', "ball", 6.35, 19, 6.35),
  tapered("tapered-r025", 0.25),
  tapered("tapered-r05", 0.5),
  tapered("tapered-r1", 1),
  generic("vbit-60", "60° V-bit", "vbit", 12.7, 11, 6.35, 60),
  generic("vbit-90", "90° V-bit", "vbit", 12.7, 6.35, 6.35, 90),
];

export function blankTool(): Tool {
  return {
    id: `tool-${Date.now().toString(36)}`,
    name: "New bit",
    kind: "flat",
    diameter: 3.175,
    flutes: 2,
    cutLength: 12,
    shank: 3.175,
    angle: null,
    taperAngle: null,
    brand: "",
    url: "",
    notes: "",
  };
}

/**
 * Picks a bit for a job from the toolbox. A relief wants a ball nose, plain or
 * tapered, with a tip sized to the artwork (roughly one sixtieth of its shorter
 * side, so detail survives without the job taking days); following paths wants
 * a flat end mill near 1/8".
 */
export function suggestTool(job: "relief" | "profile", tools: Tool[], artworkShortSide: number): Tool | null {
  const wantedDiameter = job === "relief" ? Math.min(6.35, Math.max(0.5, artworkShortSide / 60)) : 3.175;
  const candidates = tools.filter((t) =>
    job === "relief" ? t.kind === "ball" || t.kind === "taperedBall" : t.kind === "flat",
  );
  const pool = candidates.length > 0 ? candidates : tools.filter((t) => t.kind !== "vbit");
  if (pool.length === 0) return null;
  return pool.reduce((best, t) =>
    Math.abs(t.diameter - wantedDiameter) < Math.abs(best.diameter - wantedDiameter) ? t : best,
  );
}

const WORD_NUMBERS: Record<string, number> = { single: 1, one: 1, two: 2, double: 2, three: 3, four: 4 };

/** Reads the first length in `text`, in millimetres: `1/8"`, `0.25 in`, `1-1/2 inch`, `6mm`, `3.175 mm`. */
function readLength(text: string): number | null {
  const match = text.match(
    /(?:(\d+)[\s-]+)?(\d+)\s*\/\s*(\d+)\s*(?:"|''|in\b|inch(?:es)?\b)?|(\d*\.\d+|\d+)\s*(mm\b|millimet|"|''|in\b|inch(?:es)?\b)/,
  );
  if (!match) return null;
  if (match[2] && match[3]) {
    const inches = Number(match[1] ?? 0) + Number(match[2]) / Number(match[3]);
    return inches * 25.4;
  }
  const value = Number(match[4]);
  return match[5].startsWith("m") ? value : value * 25.4;
}

/** The first length that follows any of `labels`, within a short distance. */
function lengthAfter(text: string, labels: string[]): number | null {
  for (const label of labels) {
    const at = text.indexOf(label);
    if (at < 0) continue;
    const found = readLength(text.slice(at + label.length, at + label.length + 40));
    if (found !== null && found > 0 && found < 400) return found;
  }
  return null;
}

export interface ParsedTool {
  tool: Partial<Tool>;
  /** Names of the details that were recognised, for showing to the user. */
  found: string[];
}

/**
 * Picks a bit's details out of product text, such as a listing's title and
 * specification table. This is pattern matching, not understanding: the
 * result must be checked against the product.
 */
export function parseToolText(raw: string, title = ""): ParsedTool {
  const text = `${title}\n${raw}`
    .toLowerCase()
    .replace(/[“”″]/g, '"')
    .replace(/[‘’′]/g, "'")
    .replace(/½/g, " 1/2")
    .replace(/¼/g, " 1/4")
    .replace(/⅛/g, " 1/8")
    .replace(/⅜/g, " 3/8")
    .replace(/¾/g, " 3/4");
  const tool: Partial<Tool> = {};
  const found: string[] = [];
  const set = <K extends keyof Tool>(key: K, value: Tool[K] | null, label: string) => {
    if (value === null || value === undefined) return;
    tool[key] = value;
    found.push(label);
  };
  const round = (v: number | null) => (v === null ? null : Math.round(v * 1000) / 1000);

  const ball = /ball[\s-]*nose|ballnose|round[\s-]*nose|ball end/.test(text);
  const kind: ToolKind | null = ball && /taper/.test(text)
    ? "taperedBall"
    : ball
    ? "ball"
    : /v[\s-]*(bit|groove|carv)|engraving bit|chamfer/.test(text)
      ? "vbit"
      : /end[\s-]*mill|flat|fishtail|upcut|downcut|up[\s-]cut|down[\s-]cut|compression|straight/.test(text)
        ? "flat"
        : null;
  set("kind", kind, "type");

  // A tapered ball nose is sized by its tip: "0.5mm tip diameter", "0.25mm radius" or "R0.25".
  const tipDiameter = text.match(/(\d*\.?\d+)\s*mm\s*tip(?:\s*dia)?/);
  const tipRadius = text.match(/(\d*\.?\d+)\s*mm\s*(?:tip\s*)?radius|\br\s?(\d*\.\d+)\b/);
  const tip = tipDiameter ? Number(tipDiameter[1]) : tipRadius ? Number(tipRadius[1] ?? tipRadius[2]) * 2 : null;
  if (tool.kind === "taperedBall" && tip !== null && tip > 0 && tip < 10) {
    set("diameter", round(tip), "tip diameter");
  } else
  set(
    "diameter",
    round(
      lengthAfter(text, ["cutting diameter", "cut diameter", "cutter diameter", "cutting dia", "head diameter", "blade diameter", "ced", "diameter"]) ??
        // Listings often lead with the size, as in `1/8" ball nose`.
        readLength(text.slice(0, 120)),
    ),
    "diameter",
  );
  set("shank", round(lengthAfter(text, ["shank diameter", "shank dia", "shank size", "shank", "shk"])), "shank");
  set(
    "cutLength",
    round(lengthAfter(text, ["cutting length", "cutting edge length", "cut length", "flute length", "length of cut", "cutting height", "cutting depth", "loc", "cel"])),
    "cutting length",
  );

  const flutes = text.match(/(\d+|single|one|two|double|three|four)[\s-]*flutes?\b|flutes?\s*[:\-–]?\s*(\d+)/);
  if (flutes) {
    const word = flutes[1] ?? flutes[2];
    const count = WORD_NUMBERS[word] ?? Number(word);
    if (count >= 1 && count <= 8) set("flutes", count, "flutes");
  }

  if (tool.kind === "taperedBall") {
    const side = text.match(/(\d{1,2}(?:\.\d+)?)\s*(?:°|deg)/);
    if (side && Number(side[1]) > 0 && Number(side[1]) < 45) {
      set("taperAngle", Number(side[1]), "taper angle");
    } else if (tool.diameter && tool.shank && tool.cutLength) {
      set("taperAngle", taperFromDimensions(tool.diameter, tool.shank, tool.cutLength), "taper angle (from the dimensions)");
    }
  }

  if (tool.kind === "vbit") {
    const angle = text.match(/(\d{2,3})\s*(?:°|deg)/);
    if (angle && Number(angle[1]) >= 10 && Number(angle[1]) <= 150) set("angle", Number(angle[1]), "angle");
  }

  const name = title.trim().split(/\s[|–—-]\s|\n/)[0].slice(0, 80).trim();
  if (name) set("name", name, "name");
  return { tool, found };
}
