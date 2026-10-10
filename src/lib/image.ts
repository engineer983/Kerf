import type { SourceImage } from "../state/project";

export const ACCEPTED_IMAGE_TYPES = "image/png,image/jpeg,image/webp,image/bmp,image/gif";

export function isAcceptedImage(file: File): boolean {
  return ACCEPTED_IMAGE_TYPES.split(",").includes(file.type);
}

export function loadImageFile(file: File): Promise<SourceImage> {
  return new Promise((resolve, reject) => {
    const url = URL.createObjectURL(file);
    const img = new Image();
    img.onload = () =>
      resolve({
        name: file.name,
        url,
        width: img.naturalWidth,
        height: img.naturalHeight,
        bytes: file.size,
        photo: looksLikePhoto(img),
      });
    img.onerror = () => {
      URL.revokeObjectURL(url);
      reject(new Error(`Could not read ${file.name} as an image`));
    };
    img.src = url;
  });
}

/**
 * Whether an image is continuous-tone, like a photograph, as opposed to line
 * art or a logo: a good share of its pixels are neither near black nor near white.
 */
function looksLikePhoto(img: HTMLImageElement): boolean {
  const size = 64;
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = size;
  const ctx = canvas.getContext("2d", { willReadFrequently: true });
  if (!ctx) return false;
  ctx.fillStyle = "#fff";
  ctx.fillRect(0, 0, size, size);
  ctx.drawImage(img, 0, 0, size, size);
  const { data } = ctx.getImageData(0, 0, size, size);
  let midtones = 0;
  for (let i = 0; i < data.length; i += 4) {
    const luma = 0.299 * data[i] + 0.587 * data[i + 1] + 0.114 * data[i + 2];
    if (luma > 48 && luma < 208) midtones++;
  }
  return midtones / (size * size) > 0.35;
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
