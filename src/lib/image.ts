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
      });
    img.onerror = () => {
      URL.revokeObjectURL(url);
      reject(new Error(`Could not read ${file.name} as an image`));
    };
    img.src = url;
  });
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
