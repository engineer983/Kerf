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
