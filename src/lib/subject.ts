/**
 * Finding the subject of a picture, so that its background can be removed.
 *
 * The model (MODNet, Apache-2.0) runs on this computer. It is downloaded from
 * Hugging Face the first time and kept in the browser cache; the picture is
 * never sent anywhere. It was made for photographs of people and does well on
 * other things that stand clear of what is behind them. Where the subject and
 * the background are alike (white on white, say) it loses part of the subject,
 * which is why the result can be corrected by hand.
 */
import { downloading, transformers } from "./models";

const MODEL = "Xenova/modnet";

interface Cut {
  width: number;
  height: number;
  channels: number;
  data: Uint8ClampedArray | Uint8Array;
}

type Remover = (image: string) => Promise<Cut | Cut[]>;

let remover: Promise<Remover> | null = null;

function load(onProgress: (text: string) => void): Promise<Remover> {
  remover ??= (async () => {
    const { pipeline } = await transformers();
    const loaded = await pipeline("background-removal", MODEL, {
      dtype: "fp32",
      progress_callback: downloading("Downloading the model that finds the subject…", onProgress),
    });
    return loaded as unknown as Remover;
  })();
  // A failed load (no network, say) must not be remembered as the result.
  remover.catch(() => (remover = null));
  return remover;
}

/** Which part of a picture is kept: one value for each pixel, 255 kept, 0 removed. */
export interface SubjectMask {
  width: number;
  height: number;
  mask: Uint8ClampedArray;
}

/**
 * Works out which part of the picture is its subject. The mask is 255 on the
 * subject, 0 on the background, and in between along soft edges such as hair.
 */
export async function findSubject(imageUrl: string, onProgress: (text: string) => void): Promise<SubjectMask> {
  onProgress("Loading the model that finds the subject…");
  const run = await load(onProgress);
  onProgress("Finding the subject…");
  const made = await run(imageUrl);
  const cut = Array.isArray(made) ? made[0] : made;
  if (!cut || cut.channels !== 4) throw new Error("The subject could not be found in this picture.");
  const mask = new Uint8ClampedArray(cut.width * cut.height);
  for (let i = 0; i < mask.length; i++) mask[i] = cut.data[i * 4 + 3];
  return { width: cut.width, height: cut.height, mask };
}

/** A mask as a canvas: opaque where the picture is kept, clear where it is removed. */
export function maskCanvas({ width, height, mask }: SubjectMask): HTMLCanvasElement {
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const pixels = new ImageData(width, height);
  for (let i = 0; i < mask.length; i++) {
    pixels.data[i * 4] = pixels.data[i * 4 + 1] = pixels.data[i * 4 + 2] = 255;
    pixels.data[i * 4 + 3] = mask[i];
  }
  canvas.getContext("2d")!.putImageData(pixels, 0, 0);
  return canvas;
}

export type Backdrop = "white" | "black";

/**
 * The picture with everything outside the mask painted over, as a PNG file of
 * the picture's own size. The mask (opaque where the picture is kept) may be
 * any size; it is stretched over the picture.
 */
export async function cutOut(imageUrl: string, mask: HTMLCanvasElement, backdrop: Backdrop): Promise<Blob> {
  const picture = new Image();
  picture.src = imageUrl;
  await picture.decode();
  const canvas = document.createElement("canvas");
  canvas.width = picture.naturalWidth;
  canvas.height = picture.naturalHeight;
  const context = canvas.getContext("2d")!;
  context.drawImage(picture, 0, 0);
  context.globalCompositeOperation = "destination-in";
  context.imageSmoothingQuality = "high";
  context.drawImage(mask, 0, 0, canvas.width, canvas.height);
  context.globalCompositeOperation = "destination-over";
  context.fillStyle = backdrop === "white" ? "#ffffff" : "#000000";
  context.fillRect(0, 0, canvas.width, canvas.height);
  const blob = await new Promise<Blob | null>((done) => canvas.toBlob(done, "image/png"));
  if (!blob) throw new Error("The picture could not be made.");
  return blob;
}
