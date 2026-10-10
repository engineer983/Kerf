export type Rgb = [number, number, number];

export interface Material {
  id: string;
  label: string;
  /** Colour of the untouched surface. */
  base: Rgb;
  /** How the surface is textured when drawn. */
  grain: "wood" | "speckle" | "none";
  /** Colour of a full-power laser mark. Lighter than `base` on dark materials. */
  engrave: Rgb;
  /** A typical sheet thickness in millimetres, used as the default. */
  thickness: number;
}

// Colours are for preview only; they are not cutting settings.
export const MATERIALS: Material[] = [
  { id: "birch-ply", label: "Birch plywood", base: [222, 196, 150], grain: "wood", engrave: [70, 42, 20], thickness: 3 },
  { id: "basswood", label: "Basswood", base: [232, 214, 178], grain: "wood", engrave: [84, 52, 26], thickness: 3 },
  { id: "maple", label: "Maple", base: [230, 205, 165], grain: "wood", engrave: [78, 48, 24], thickness: 19 },
  { id: "cherry", label: "Cherry", base: [176, 112, 74], grain: "wood", engrave: [52, 28, 16], thickness: 19 },
  { id: "walnut", label: "Walnut", base: [110, 74, 48], grain: "wood", engrave: [36, 22, 14], thickness: 19 },
  { id: "pine", label: "Pine", base: [226, 190, 128], grain: "wood", engrave: [80, 46, 20], thickness: 19 },
  { id: "bamboo", label: "Bamboo", base: [214, 186, 120], grain: "wood", engrave: [74, 46, 20], thickness: 5 },
  { id: "mdf", label: "MDF", base: [188, 160, 120], grain: "speckle", engrave: [62, 40, 22], thickness: 6 },
  { id: "cardboard", label: "Cardboard", base: [176, 142, 100], grain: "speckle", engrave: [66, 44, 26], thickness: 3 },
  { id: "leather", label: "Leather", base: [140, 90, 50], grain: "speckle", engrave: [56, 32, 16], thickness: 2 },
  { id: "black-acrylic", label: "Black acrylic", base: [28, 28, 30], grain: "none", engrave: [190, 190, 196], thickness: 3 },
  { id: "white-acrylic", label: "White acrylic", base: [240, 240, 238], grain: "none", engrave: [150, 150, 150], thickness: 3 },
  { id: "slate", label: "Slate", base: [58, 62, 66], grain: "speckle", engrave: [172, 174, 178], thickness: 6 },
  { id: "anodized", label: "Anodised aluminium", base: [34, 36, 40], grain: "none", engrave: [226, 228, 232], thickness: 2 },
  { id: "foam", label: "Rigid foam", base: [236, 172, 182], grain: "speckle", engrave: [150, 96, 104], thickness: 25 },
];

export const DEFAULT_MATERIAL = MATERIALS[0];

export function findMaterial(id: string): Material {
  return MATERIALS.find((m) => m.id === id) ?? DEFAULT_MATERIAL;
}

export const css = ([r, g, b]: Rgb, alpha = 1) => `rgba(${r}, ${g}, ${b}, ${alpha})`;

/** A small deterministic random generator, so a material always looks the same. */
function random(seed: number) {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Paints the bare material over a whole canvas. `pxPerMm` keeps the grain to scale. */
export function paintMaterial(canvas: HTMLCanvasElement, material: Material, pxPerMm: number) {
  const ctx = canvas.getContext("2d")!;
  const { width, height } = canvas;
  ctx.globalAlpha = 1;
  ctx.fillStyle = css(material.base);
  ctx.fillRect(0, 0, width, height);
  const next = random(material.id.length * 7919 + material.base[0]);

  if (material.grain === "wood") {
    // Long, slightly wavy streaks running along the sheet.
    const streaks = Math.max(24, Math.round(height / pxPerMm / 0.8));
    for (let i = 0; i < streaks; i++) {
      const y = next() * height;
      const darker = next() < 0.7;
      ctx.strokeStyle = darker ? `rgba(60, 30, 5, ${0.03 + next() * 0.09})` : `rgba(255, 245, 220, ${0.03 + next() * 0.08})`;
      ctx.lineWidth = Math.max(1, (0.2 + next() * 1.4) * pxPerMm);
      const wave = (1 + next() * 3) * pxPerMm;
      const period = (60 + next() * 160) * pxPerMm;
      const phase = next() * Math.PI * 2;
      ctx.beginPath();
      for (let x = 0; x <= width; x += Math.max(8, width / 60)) {
        const yy = y + Math.sin(x / period + phase) * wave;
        if (x === 0) ctx.moveTo(x, yy);
        else ctx.lineTo(x, yy);
      }
      ctx.stroke();
    }
  } else if (material.grain === "speckle") {
    const dots = Math.min(40000, Math.round((width * height) / 60));
    for (let i = 0; i < dots; i++) {
      ctx.fillStyle = next() < 0.5 ? `rgba(0, 0, 0, ${next() * 0.1})` : `rgba(255, 255, 255, ${next() * 0.08})`;
      const size = Math.max(1, next() * 0.6 * pxPerMm);
      ctx.fillRect(next() * width, next() * height, size, size);
    }
  }
}
