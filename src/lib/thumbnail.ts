/** The finished-look picture currently on the canvas, if a job has been generated. */
let resultCanvas: HTMLCanvasElement | null = null;

/** Called by the canvas whenever its simulation changes. */
export function setResultCanvas(canvas: HTMLCanvasElement | null) {
  resultCanvas = canvas;
}

const THUMBNAIL_PX = 360;

/**
 * A small picture of the project for the gallery: the simulated result when
 * there is one, otherwise the imported image. Returned as a JPEG data URL.
 */
export async function makeThumbnail(imageUrl: string): Promise<string> {
  let source: CanvasImageSource & { width: number; height: number };
  if (resultCanvas && resultCanvas.width > 0) {
    source = resultCanvas;
  } else {
    const picture = new Image();
    picture.src = imageUrl;
    await picture.decode();
    source = picture;
  }
  const scale = Math.min(1, THUMBNAIL_PX / Math.max(source.width, source.height));
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(source.width * scale));
  canvas.height = Math.max(1, Math.round(source.height * scale));
  const ctx = canvas.getContext("2d")!;
  ctx.fillStyle = "#fff";
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  ctx.drawImage(source, 0, 0, canvas.width, canvas.height);
  return canvas.toDataURL("image/jpeg", 0.82);
}
