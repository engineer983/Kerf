import * as ipc from "./ipc";
import { useProject, type SourceImage } from "../state/project";

/** Longest side, in pixels, kept of a picture that was made or downloaded. */
const MAX_PICTURE_PX = 3000;

/** A name that is safe to use for a file and reads well as a project's name. */
export function pictureName(text: unknown, fallback: string): string {
  const name = typeof text === "string" ? text.replace(/[\\/:*?"<>|]+/g, " ").replace(/\s+/g, " ").trim().slice(0, 60) : "";
  return name || fallback;
}

/**
 * Turns an image file's bytes, in whatever format the browser can read, into
 * a PNG file. Drawing it afresh means only pixels are kept from a download.
 */
export async function toPngFile(bytes: ArrayBuffer, name: string): Promise<File> {
  const url = URL.createObjectURL(new Blob([bytes]));
  try {
    const picture = new Image();
    picture.src = url;
    try {
      await picture.decode();
    } catch {
      throw new Error("What came back is not a picture this app can read.");
    }
    const scale = Math.min(1, MAX_PICTURE_PX / Math.max(picture.naturalWidth, picture.naturalHeight));
    const canvas = document.createElement("canvas");
    canvas.width = Math.max(1, Math.round(picture.naturalWidth * scale));
    canvas.height = Math.max(1, Math.round(picture.naturalHeight * scale));
    const ctx = canvas.getContext("2d")!;
    // Transparent areas become white, as they are treated everywhere else.
    ctx.fillStyle = "#fff";
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    ctx.drawImage(picture, 0, 0, canvas.width, canvas.height);
    const png = await new Promise<Blob | null>((done) => canvas.toBlob(done, "image/png"));
    if (!png) throw new Error("The picture could not be prepared.");
    return new File([png], `${name}.png`, { type: "image/png" });
  } finally {
    URL.revokeObjectURL(url);
  }
}

/** Saves what was being worked on, then loads `file` as a new project. */
export async function loadAsNewProject(file: File, options?: { drawnHeightMap?: boolean }): Promise<SourceImage> {
  const before = useProject.getState();
  if (before.image) await before.saveProject();
  await useProject.getState().importFile(file, options);
  const after = useProject.getState();
  if (after.error || !after.image || after.image === before.image) {
    throw new Error(after.error ?? "The picture could not be loaded.");
  }
  return after.image;
}

/** Who made a found picture and how it may be used, in a sentence. */
export function creditLine(found: ipc.FoundPicture): string {
  const by = found.creator ? ` by ${found.creator}` : "";
  return `"${found.title}"${by}, ${found.licence}${found.needsCredit ? " (credit the creator if you share or sell the piece)" : ""}`;
}

/** Downloads a picture a search found and loads it as a new project. */
export async function loadFoundPicture(found: ipc.FoundPicture): Promise<SourceImage> {
  const bytes = await ipc.fetchPicture(found.url);
  return loadAsNewProject(await toPngFile(bytes, pictureName(found.title, "found picture")));
}
