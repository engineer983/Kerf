/**
 * The camera over the bed: choosing one, taking a still, and asking the
 * engine to line it up with the bed.
 *
 * The window opens the camera itself (the same code on desktop and in a
 * browser), keeps it open only long enough for one picture, and closes it.
 *
 * These calls are kept out of `ipc.ts` on purpose: the assistant's tools use
 * `ipc`, and the assistant must have no way to take or see camera pictures.
 */
import { invoke } from "@tauri-apps/api/core";

/** One marker: a place in the still (pixels) and where it is on the bed (canvas mm: X from the left, Y down from the back). */
export interface FitPoint {
  px: number;
  py: number;
  x: number;
  y: number;
}

/** How a camera's pictures line up with the bed. */
export interface CameraFit {
  points: [FitPoint, FitPoint, FitPoint, FitPoint];
  /** The bed the fit was made for; it is no good for another size. */
  bedWidth: number;
  bedHeight: number;
  /** The still the markers were placed on. */
  pictureWidth: number;
  pictureHeight: number;
}

export interface CameraInfo {
  deviceId: string;
  label: string;
}

/** How long to let the camera settle its exposure before taking the still. */
const SETTLE_MS = 300;

const isMac = typeof navigator !== "undefined" && /Mac/i.test(navigator.platform || navigator.userAgent);
const isLinux = typeof navigator !== "undefined" && /Linux/i.test(navigator.userAgent) && !/Android/i.test(navigator.userAgent);

function media(): MediaDevices {
  const devices = typeof navigator !== "undefined" ? navigator.mediaDevices : undefined;
  if (!devices?.getUserMedia) {
    throw (
      "This window can't use a camera. In a browser it needs a secure page (https, or localhost)" +
      (isMac ? "; on a Mac it needs macOS 12 or later." : "; on the desktop it needs a newer system webview.")
    );
  }
  return devices;
}

/** Turns a camera failure into something the person can act on. */
export function cameraError(e: unknown): string {
  if (typeof e === "string") return e;
  const name = e instanceof DOMException || e instanceof Error ? e.name : "";
  const privacy = isMac
    ? " On a Mac, look in System Settings > Privacy & Security > Camera."
    : isLinux
      ? " On Linux, check that your user is in the video group."
      : " On Windows, check Settings > Privacy & security > Camera, including \"Let desktop apps access your camera\".";
  switch (name) {
    case "NotFoundError":
    case "OverconstrainedError":
      return "No camera was found. Check it is plugged in, then try again.";
    case "NotAllowedError":
    case "SecurityError":
    case "NotReadableError":
    case "AbortError":
      return `Kerf couldn't use the camera. Check the camera privacy settings, or close any other program using it.${privacy}`;
    default:
      return e instanceof Error ? e.message : "The camera could not be used.";
  }
}

function stopAll(stream: MediaStream | null) {
  stream?.getTracks().forEach((track) => track.stop());
}

/**
 * The cameras on this computer. Their names are hidden until the camera has
 * been allowed once, so this asks first (opening and at once closing one).
 */
export async function listCameras(): Promise<CameraInfo[]> {
  const devices = media();
  let found = (await devices.enumerateDevices()).filter((d) => d.kind === "videoinput");
  if (found.length > 0 && found.every((d) => !d.label)) {
    let stream: MediaStream | null = null;
    try {
      stream = await devices.getUserMedia({ video: true, audio: false });
    } finally {
      stopAll(stream);
    }
    found = (await devices.enumerateDevices()).filter((d) => d.kind === "videoinput");
  }
  return found.map((d, i) => ({ deviceId: d.deviceId, label: d.label || `Camera ${i + 1}` }));
}

/** Waits for the video to show a real frame, then a little longer for the exposure. */
async function firstFrame(video: HTMLVideoElement): Promise<void> {
  await new Promise<void>((resolve, reject) => {
    const timeout = window.setTimeout(() => reject(new Error("The camera didn't send a picture. Try again, or choose another camera.")), 8000);
    const done = () => {
      window.clearTimeout(timeout);
      resolve();
    };
    // Not every webview has requestVideoFrameCallback; older ones get the first loaded frame.
    const frames = video as HTMLVideoElement & { requestVideoFrameCallback?: (cb: () => void) => number };
    if (typeof frames.requestVideoFrameCallback === "function") frames.requestVideoFrameCallback(done);
    else video.addEventListener("loadeddata", done, { once: true });
  });
  await new Promise((resolve) => window.setTimeout(resolve, SETTLE_MS));
}

/** Opens the camera, takes one still as a JPEG, and closes the camera straight away. */
export async function takeStill(deviceId: string | null): Promise<Blob> {
  const devices = media();
  let stream: MediaStream | null = null;
  try {
    stream = await devices.getUserMedia({
      audio: false,
      video: {
        ...(deviceId ? { deviceId: { exact: deviceId } } : {}),
        width: { ideal: 1920 },
        height: { ideal: 1080 },
      },
    });
    const video = document.createElement("video");
    video.muted = true;
    video.playsInline = true;
    // Kept as a stream object, never a blob address, so the window's
    // content rules need no change.
    video.srcObject = stream;
    await video.play();
    await firstFrame(video);
    const canvas = document.createElement("canvas");
    canvas.width = video.videoWidth;
    canvas.height = video.videoHeight;
    if (!canvas.width || !canvas.height) throw new Error("The camera sent an empty picture. Try again.");
    canvas.getContext("2d")!.drawImage(video, 0, 0);
    video.srcObject = null;
    return await new Promise<Blob>((resolve, reject) =>
      canvas.toBlob((blob) => (blob ? resolve(blob) : reject(new Error("The picture could not be made."))), "image/jpeg", 0.9),
    );
  } finally {
    // The camera light goes off as soon as the picture is taken.
    stopAll(stream);
  }
}

/** Gives the engine the latest still; it keeps it for redrawing. Returns its size in pixels. */
export async function setCameraStill(still: Blob): Promise<{ width: number; height: number }> {
  // The browser version forwards only bytes, not a Blob.
  return invoke("set_camera_still", new Uint8Array(await still.arrayBuffer()));
}

/** Checks a fit can be used, and whether it looks mirrored. */
export function checkCameraFit(fit: CameraFit): Promise<{ mirrored: boolean }> {
  return invoke("check_camera_fit", { fit });
}

/** The latest still redrawn as the bed seen from above, as a PNG file. */
export function cameraBedPicture(fit: CameraFit, bed: { width: number; height: number }): Promise<ArrayBuffer> {
  return invoke("camera_bed_picture", { fit, bed });
}

/** The material as found in the camera picture, in canvas bed mm (X from the left, Y down from the back). */
export interface FoundMaterial {
  /** Back left, back right, front right, front left (before any turn). */
  corners: [[number, number], [number, number], [number, number], [number, number]];
  centre: [number, number];
  /** Along the side nearest to left-to-right. */
  width: number;
  /** Along the side nearest to back-to-front. */
  length: number;
  /** How far it is turned, clockwise on screen. */
  angleDeg: number;
  /** The largest rectangle square to the bed inside it, round the same centre. */
  straightWidth: number;
  straightLength: number;
  /** What to check, in plain words. Empty when nothing looks doubtful. */
  warnings: string[];
}

export type Corners = FoundMaterial["corners"];

/**
 * The rectangle that four corners (in order round it) stand for: the centre
 * is their average and each side the average of the two opposite sides.
 * Used once the person has dragged the corners of a found outline.
 */
export function outlineRect(corners: Corners) {
  const [a, b, c, d] = corners;
  const centre: [number, number] = [(a[0] + b[0] + c[0] + d[0]) / 4, (a[1] + b[1] + c[1] + d[1]) / 4];
  const side = (p: [number, number], q: [number, number]) => Math.hypot(q[0] - p[0], q[1] - p[1]);
  // The two sides that run across, added, give the direction of the turn.
  const across = [b[0] - a[0] + (c[0] - d[0]), b[1] - a[1] + (c[1] - d[1])];
  return {
    centre,
    width: (side(a, b) + side(d, c)) / 2,
    length: (side(a, d) + side(b, c)) / 2,
    angleDeg: (Math.atan2(across[1], across[0]) * 180) / Math.PI,
  };
}

/**
 * The largest rectangle square to the bed, round the same centre, inside a
 * `width` × `length` rectangle turned by `angleDeg`. The same sum as
 * `straight_inside` in `kerf-core/src/material.rs`, where it is tested.
 */
export function straightInside(width: number, length: number, angleDeg: number): [number, number] {
  const s = Math.abs(Math.sin((angleDeg * Math.PI) / 180));
  const c = Math.abs(Math.cos((angleDeg * Math.PI) / 180));
  if (s < 1e-9) return [width, length];
  if (c < 1e-9) return [length, width];
  const [hw, hl] = [width / 2, length / 2];
  const fits = (a: number, b: number) => a >= 0 && b >= 0 && a * c + b * s <= hw + 1e-9 && a * s + b * c <= hl + 1e-9;
  let best: [number, number] = [0, 0];
  const consider = (a: number, b: number) => {
    if (fits(a, b) && a * b > best[0] * best[1]) best = [a, b];
  };
  consider(hw / (2 * c), hw / (2 * s));
  consider(hl / (2 * s), hl / (2 * c));
  const det = c * c - s * s;
  if (Math.abs(det) > 1e-12) consider((hw * c - hl * s) / det, (hl * c - hw * s) / det);
  return [best[0] * 2, best[1] * 2];
}

/**
 * Looks for the material in the latest still. It only proposes an outline;
 * nothing is changed until the person says to use it.
 */
export function findMaterial(request: {
  fit: CameraFit;
  bed: { width: number; height: number };
  /** A place the person clicked on the material, in bed mm. */
  seed: [number, number] | null;
  /** Where the material is set to be now, in bed mm. */
  stock: { x: number; y: number; width: number; height: number } | null;
  thickness: number;
}): Promise<FoundMaterial> {
  return invoke("find_material", request);
}
