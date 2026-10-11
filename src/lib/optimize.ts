import type { Preprocess, TraceSettings } from "./ipc";

// Tuning a picture for what is being made from it. Nothing here redraws the
// picture: it measures how light and dark it is, and chooses the brightness,
// contrast, smoothing and trace settings that suit the job.

/** What the picture is being used for. */
export type Purpose = "engraving" | "carving" | "shapes";

export const PURPOSE_LABELS: Record<Purpose, string> = {
  engraving: "engraving with shading",
  carving: "3D carving",
  shapes: "tracing into shapes",
};

export interface Tones {
  /** Share of pixels at each of 256 brightness levels. */
  histogram: number[];
  /** The picture's longest side, in pixels. */
  longSide: number;
}

/** Longest side of the copy that is measured; plenty for a histogram. */
const SAMPLE_PX = 256;

/** Measures a picture's brightness levels, with transparency as white. */
export async function measureTones(url: string): Promise<Tones> {
  const picture = new Image();
  picture.src = url;
  await picture.decode();
  const scale = Math.min(1, SAMPLE_PX / Math.max(picture.naturalWidth, picture.naturalHeight));
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(picture.naturalWidth * scale));
  canvas.height = Math.max(1, Math.round(picture.naturalHeight * scale));
  const ctx = canvas.getContext("2d", { willReadFrequently: true })!;
  ctx.fillStyle = "#fff";
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  ctx.drawImage(picture, 0, 0, canvas.width, canvas.height);
  const { data } = ctx.getImageData(0, 0, canvas.width, canvas.height);
  const histogram = new Array<number>(256).fill(0);
  for (let i = 0; i < data.length; i += 4) {
    histogram[Math.round(0.299 * data[i] + 0.587 * data[i + 1] + 0.114 * data[i + 2])]++;
  }
  const pixels = data.length / 4;
  return { histogram: histogram.map((n) => n / pixels), longSide: Math.max(picture.naturalWidth, picture.naturalHeight) };
}

/** The brightness, 0 to 1, below which `share` of the picture lies. */
function percentile(histogram: number[], share: number): number {
  let seen = 0;
  for (let level = 0; level < 256; level++) {
    seen += histogram[level];
    if (seen >= share) return level / 255;
  }
  return 1;
}

/** Otsu's method, as the tracer uses: the level that best splits dark from light. */
function splitLevel(histogram: number[]): number {
  const mean = histogram.reduce((sum, share, level) => sum + share * level, 0);
  let best = 0;
  let bestVariance = 0;
  let dark = 0;
  let darkSum = 0;
  for (let level = 0; level < 256; level++) {
    dark += histogram[level];
    darkSum += histogram[level] * level;
    const light = 1 - dark;
    if (dark === 0 || light <= 0) continue;
    const variance = dark * light * (darkSum / dark - (mean - darkSum) / light) ** 2;
    if (variance > bestVariance) {
      bestVariance = variance;
      best = level;
    }
  }
  return best;
}

const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));
const round = (value: number, places: number) => Number(value.toFixed(places));

/**
 * Brightness and contrast that stretch the picture's tones to fill the range
 * from black to white, ignoring `ignored` of the pixels at each end. They are
 * in the form the tracer applies: output = (input - 0.5) x gain + 0.5 + brightness,
 * with gain = (1 + contrast) / (1 - contrast).
 */
function fillRange(histogram: number[], ignored: number): Pick<Preprocess, "brightness" | "contrast"> {
  const low = percentile(histogram, ignored);
  const high = percentile(histogram, 1 - ignored);
  // A picture that is nearly one tone has nothing to stretch.
  if (high - low < 0.05) return { brightness: 0, contrast: 0 };
  // Never flattened, and never stretched so far that noise becomes the picture.
  const gain = clamp(0.96 / (high - low), 1, 3);
  const middle = (low + high) / 2;
  return {
    contrast: round((gain - 1) / (gain + 1), 2),
    brightness: round(clamp(-(middle - 0.5) * gain, -0.5, 0.5), 2),
  };
}

export interface Optimised {
  purpose: Purpose;
  settings: TraceSettings;
  /** What was changed, for telling the person. */
  changes: string[];
}

/** The settings that suit `purpose` for a picture with these tones. */
export function optimise(purpose: Purpose, tones: Tones, current: TraceSettings): Optimised {
  const before = current.preprocess;
  let preprocess: Preprocess;
  let settings: TraceSettings;

  if (purpose === "shapes") {
    // The dividing line between ink and paper is found afresh, so the tones
    // are left as they are. A little smoothing takes the stair-steps off
    // edges, more so on a small picture where each pixel is a bigger step.
    const level = splitLevel(tones.histogram);
    const darkShare = tones.histogram.slice(0, level + 1).reduce((a, b) => a + b, 0);
    preprocess = {
      brightness: 0,
      contrast: 0,
      blurSigma: tones.longSide < 600 ? 0.4 : tones.longSide < 1600 ? 0.7 : 1,
      threshold: { mode: "otsu" },
      // Mostly dark means a light subject on a dark ground: trace the light.
      invert: darkShare > 0.6,
    };
    settings = { ...current, mode: "outline", preprocess, speckle: clamp(Math.round(tones.longSide / 350), 2, 8), curves: true };
  } else {
    // Shading and height both come from the tones, so they should use the
    // whole range. A carving is also smoothed, since single-pixel noise would
    // be cut as pits and spikes; an engraving keeps all its detail.
    const blurSigma = purpose === "carving" ? round(clamp(tones.longSide / 1500, 0.5, 2), 1) : 0;
    preprocess = { ...before, ...fillRange(tones.histogram, purpose === "carving" ? 0.005 : 0.01), blurSigma };
    settings = { ...current, preprocess };
  }

  const changes: string[] = [];
  const percent = (value: number) => (value === 0 ? "back to normal" : `${value > 0 ? "+" : ""}${Math.round(value * 100)}%`);
  if (preprocess.contrast !== before.contrast) changes.push(`contrast ${percent(preprocess.contrast)}`);
  if (preprocess.brightness !== before.brightness) changes.push(`brightness ${percent(preprocess.brightness)}`);
  if (preprocess.blurSigma !== before.blurSigma) changes.push(preprocess.blurSigma ? `blur ${preprocess.blurSigma} px` : "blur off");
  if (purpose === "shapes") {
    if (before.threshold.mode !== "otsu") changes.push("automatic threshold");
    if (preprocess.invert !== before.invert) changes.push(preprocess.invert ? "tracing the light areas" : "tracing the dark areas");
    if (settings.speckle !== current.speckle) changes.push(`specks under ${settings.speckle} px ignored`);
    if (settings.mode !== current.mode) changes.push("outline tracing");
    if (!current.curves) changes.push("smooth curves");
  }
  return { purpose, settings, changes };
}
