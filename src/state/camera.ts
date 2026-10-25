import { create } from "zustand";
import {
  cameraBedPicture,
  cameraError,
  findMaterial as detectMaterial,
  listCameras,
  outlineRect,
  setCameraStill,
  straightInside,
  takeStill,
  type CameraFit,
  type CameraInfo,
  type Corners,
  type FoundMaterial,
} from "../lib/camera";
import { useMachine } from "./machine";
import { placeOf, stockRect, useProject } from "./project";

/** Sizes within this of what was typed keep the typed size: a ruler beats a camera. */
const KEEP_SIZE_MM = 10;
/** Turned by more than this, only the straight part in the middle is used. */
const STRAIGHT_ABOVE_DEG = 1;

/** Camera settings kept between sessions. The picture itself never is. */
const CAMERA_KEY = "kerf.camera";

interface StoredCamera {
  deviceId: string | null;
  /** The camera the fit was made with; another camera needs lining up again. */
  fitDeviceId: string | null;
  fit: CameraFit | null;
  /** When the fit was made, so a camera knocked since is easier to suspect. */
  linedUpAt: string | null;
  show: boolean;
  opacity: number;
}

function stored(): StoredCamera {
  const empty: StoredCamera = { deviceId: null, fitDeviceId: null, fit: null, linedUpAt: null, show: true, opacity: 0.7 };
  try {
    const saved = JSON.parse(localStorage.getItem(CAMERA_KEY) ?? "null") as Partial<StoredCamera> | null;
    if (!saved || typeof saved !== "object") return empty;
    return {
      deviceId: typeof saved.deviceId === "string" ? saved.deviceId : null,
      fitDeviceId: typeof saved.fitDeviceId === "string" ? saved.fitDeviceId : null,
      fit: saved.fit && Array.isArray(saved.fit.points) && saved.fit.points.length === 4 ? saved.fit : null,
      linedUpAt: typeof saved.linedUpAt === "string" ? saved.linedUpAt : null,
      show: saved.show !== false,
      opacity: typeof saved.opacity === "number" && saved.opacity >= 0.1 && saved.opacity <= 1 ? saved.opacity : 0.7,
    };
  } catch {
    return empty;
  }
}

function remember(s: StoredCamera) {
  try {
    localStorage.setItem(CAMERA_KEY, JSON.stringify(s));
  } catch {
    // Storage unavailable: the settings last until the window closes.
  }
}

export interface Still {
  /** An address for showing the JPEG, made in this window only. */
  url: string;
  width: number;
  height: number;
  takenAt: Date;
}

/** Why the picture can't be shown over the bed, if it can't. */
export type NotLinedUp = "never" | "camera" | "bed" | "picture" | null;

interface CameraStore extends StoredCamera {
  cameras: CameraInfo[];
  still: Still | null;
  /** The still redrawn as the bed seen from above. */
  bedPictureUrl: string | null;
  /** A job has run, or the material changed, since the picture was taken. */
  stale: boolean;
  busy: boolean;
  error: string | null;
  lineUpOpen: boolean;

  findCameras: () => Promise<void>;
  chooseCamera: (deviceId: string) => void;
  /** Takes a new still and shows it. Opens the lining-up screen if the camera hasn't been lined up. */
  capture: () => Promise<void>;
  saveFit: (fit: CameraFit) => void;
  forget: () => void;
  setShow: (show: boolean) => void;
  setOpacity: (opacity: number) => void;
  openLineUp: (open: boolean) => void;
  /** Redraws the picture to the bed, after the still, fit or bed has changed. */
  redraw: () => Promise<void>;

  /**
   * The material found in the picture, with its corners as the person has
   * moved them. Kept only with the still it came from, never in the project.
   */
  found: { result: FoundMaterial; corners: Corners; stillAt: Date } | null;
  /** The panel for checking a found outline is open. */
  confirming: boolean;
  finding: boolean;
  /** Waiting for a click on the material to start from. */
  picking: boolean;
  /** Why the material couldn't be found. */
  findError: string | null;
  /** Use the width and length typed in, rather than the camera's. */
  keepTypedSize: boolean;
  /**
   * Remind the person to frame the job first: after an outline is used
   * ("found"), or after the material is moved by typing or by the assistant ("moved").
   */
  framePrompt: "found" | "moved" | null;
  /**
   * Looks for the material, from a spot clicked on it if given. Takes a new
   * picture first when there is none or it may be out of date. Only
   * proposes an outline; nothing changes until `applyFound`.
   */
  findMaterial: (seed?: [number, number] | null) => Promise<void>;
  setCorner: (index: number, at: [number, number]) => void;
  setPicking: (picking: boolean) => void;
  setKeepTypedSize: (keep: boolean) => void;
  /** Closes the panel without using the outline. */
  cancelFound: () => void;
  /** Moves the material to the outline and centres the design on it, as one undo step. */
  applyFound: () => void;
  dismissFramePrompt: () => void;
}

/** What using an outline would set the material to, in bed mm, and whether it is the straight part of a turned sheet. */
export function outlinePlan(
  found: NonNullable<CameraStore["found"]>,
  keepTypedSize: boolean,
  typed: { stockWidth: number; stockLength: number },
) {
  const rect = outlineRect(found.corners);
  const [width, length] = keepTypedSize ? [typed.stockWidth, typed.stockLength] : [rect.width, rect.length];
  const turned = Math.abs(rect.angleDeg) > STRAIGHT_ABOVE_DEG;
  // Kerf can't turn a design, so on a turned sheet it keeps to the part square to the bed.
  const [w, l] = turned ? straightInside(width, length, rect.angleDeg) : [width, length];
  return {
    turned,
    measured: rect,
    rect: { x: rect.centre[0] - w / 2, y: rect.centre[1] - l / 2, width: w, height: l },
  };
}

/** Whether a found size is close enough to the typed one to keep the typed one. */
export function closeToTyped(found: { width: number; length: number }, typed: { stockWidth: number; stockLength: number }) {
  return Math.abs(found.width - typed.stockWidth) <= KEEP_SIZE_MM && Math.abs(found.length - typed.stockLength) <= KEEP_SIZE_MM;
}

const tenth = (mm: number) => Math.round(mm * 10) / 10;
/** True while an outline is being applied, so the move doesn't mark the picture as out of date. */
let applying = false;

/** Whether the saved fit can be used for this still and bed, and why not. */
export function notLinedUp(
  s: Pick<CameraStore, "fit" | "fitDeviceId" | "deviceId" | "still">,
  bed: { width: number; height: number },
): NotLinedUp {
  if (!s.fit) return "never";
  if (s.fitDeviceId && s.deviceId && s.fitDeviceId !== s.deviceId) return "camera";
  if (Math.abs(s.fit.bedWidth - bed.width) > 0.01 || Math.abs(s.fit.bedHeight - bed.height) > 0.01) return "bed";
  if (s.still && (s.still.width !== s.fit.pictureWidth || s.still.height !== s.fit.pictureHeight)) return "picture";
  return null;
}

export const NOT_LINED_UP_MESSAGE: Record<Exclude<NotLinedUp, null>, string> = {
  never: "Line the camera up with the bed to see the picture under the design.",
  camera: "This is a different camera from the one that was lined up. Line it up again.",
  bed: "The bed size has changed since the camera was lined up. Line it up again.",
  picture: "This camera's pictures are a different size from when it was lined up. Line it up again.",
};

export const useCamera = create<CameraStore>((set, get) => {
  const keep = () => {
    const { deviceId, fitDeviceId, fit, linedUpAt, show, opacity } = get();
    remember({ deviceId, fitDeviceId, fit, linedUpAt, show, opacity });
  };

  return {
    ...stored(),
    cameras: [],
    still: null,
    bedPictureUrl: null,
    stale: false,
    busy: false,
    error: null,
    lineUpOpen: false,

    findCameras: async () => {
      set({ busy: true, error: null });
      try {
        const cameras = await listCameras();
        const current = get().deviceId;
        const deviceId = cameras.some((c) => c.deviceId === current) ? current : (cameras[0]?.deviceId ?? null);
        set({ cameras, deviceId, busy: false, error: cameras.length ? null : "No camera was found. Check it is plugged in, then try again." });
        keep();
      } catch (e) {
        set({ busy: false, error: cameraError(e) });
      }
    },

    chooseCamera: (deviceId) => {
      set({ deviceId });
      keep();
    },

    capture: async () => {
      if (get().busy) return;
      set({ busy: true, error: null });
      try {
        const blob = await takeStill(get().deviceId);
        const size = await setCameraStill(blob);
        const old = get().still;
        if (old) URL.revokeObjectURL(old.url);
        // An outline found in the old picture says nothing about the new one.
        set({
          still: { url: URL.createObjectURL(blob), ...size, takenAt: new Date() },
          stale: false,
          busy: false,
          found: null,
          confirming: get().finding,
        });
        const bed = useProject.getState().bed;
        if (notLinedUp(get(), bed)) set({ lineUpOpen: true });
        else await get().redraw();
      } catch (e) {
        set({ busy: false, error: cameraError(e) });
      }
    },

    saveFit: (fit) => {
      set({ fit, linedUpAt: new Date().toISOString(), fitDeviceId: get().deviceId, lineUpOpen: false });
      keep();
      void get().redraw();
    },

    forget: () => {
      const { still, bedPictureUrl } = get();
      if (still) URL.revokeObjectURL(still.url);
      if (bedPictureUrl) URL.revokeObjectURL(bedPictureUrl);
      set({ deviceId: null, fitDeviceId: null, fit: null, linedUpAt: null, still: null, bedPictureUrl: null, error: null, stale: false, found: null, confirming: false, picking: false });
      keep();
    },

    setShow: (show) => {
      set({ show });
      keep();
    },

    setOpacity: (opacity) => {
      set({ opacity });
      keep();
    },

    openLineUp: (lineUpOpen) => set({ lineUpOpen }),

    redraw: async () => {
      const { still, fit, bedPictureUrl } = get();
      const bed = useProject.getState().bed;
      const clear = () => {
        if (bedPictureUrl) URL.revokeObjectURL(bedPictureUrl);
        set({ bedPictureUrl: null });
      };
      if (!still || !fit || notLinedUp(get(), bed)) {
        clear();
        return;
      }
      set({ busy: true });
      try {
        const png = await cameraBedPicture(fit, { width: bed.width, height: bed.height });
        const url = URL.createObjectURL(new Blob([png], { type: "image/png" }));
        const previous = get().bedPictureUrl;
        if (previous) URL.revokeObjectURL(previous);
        set({ bedPictureUrl: url, busy: false });
      } catch (e) {
        clear();
        set({ busy: false, error: cameraError(e) });
      }
    },

    found: null,
    confirming: false,
    finding: false,
    picking: false,
    findError: null,
    keepTypedSize: true,
    framePrompt: null,

    findMaterial: async (seed = null) => {
      if (get().finding || get().busy) return;
      const bed = useProject.getState().bed;
      // The outline is drawn over the picture, so the picture is shown.
      if (!get().show) get().setShow(true);
      // The panel opens straight away, so the person sees it working.
      set({ finding: true, findError: null, picking: false, confirming: true, framePrompt: null, found: null });
      try {
        if (!get().still || get().stale) {
          await get().capture();
          if (!get().still || get().error) throw get().error ?? "No picture could be taken.";
        }
        const { fit, still } = get();
        const reason = notLinedUp(get(), bed);
        if (!fit || !still || reason) throw NOT_LINED_UP_MESSAGE[reason ?? "never"];
        const s = useProject.getState();
        const result = await detectMaterial({
          fit,
          bed: { width: bed.width, height: bed.height },
          seed,
          stock: stockRect(s),
          thickness: s.thickness,
        });
        // A newer picture may have been taken meanwhile.
        if (get().still !== still) throw "The picture changed while Kerf was looking. Try again.";
        set({
          found: { result, corners: result.corners, stillAt: still.takenAt },
          keepTypedSize: closeToTyped(result, s),
          finding: false,
        });
      } catch (e) {
        set({ finding: false, findError: cameraError(e) });
      }
    },

    setCorner: (index, at) => {
      const found = get().found;
      if (!found) return;
      const corners = found.corners.map((c, i) => (i === index ? at : c)) as Corners;
      set({ found: { ...found, corners } });
    },

    setPicking: (picking) => set({ picking }),
    setKeepTypedSize: (keepTypedSize) => set({ keepTypedSize }),

    cancelFound: () => set({ confirming: false, picking: false, findError: null, found: null }),

    applyFound: () => {
      const { found, keepTypedSize } = get();
      if (!found) return;
      const project = useProject.getState();
      // The camera, bed or picture may have changed since the outline was
      // found; then it no longer says where anything is on the bed.
      if (notLinedUp(get(), project.bed)) {
        get().cancelFound();
        return;
      }
      const { rect } = outlinePlan(found, keepTypedSize, project);
      const place = placeOf(rect, project);
      applying = true;
      try {
        project.placeMaterial({
          stockLeft: tenth(place.left),
          stockFront: tenth(place.front),
          stockWidth: tenth(rect.width),
          stockLength: tenth(rect.height),
        });
      } finally {
        applying = false;
      }
      // The outline stays drawn as a guide while the picture is shown.
      set({ confirming: false, picking: false, framePrompt: "found" });
    },

    dismissFramePrompt: () => set({ framePrompt: null }),
  };
});

// The bed size decides whether the fit still holds, and the picture's size.
// Changing the origin corner doesn't: the fit is in canvas millimetres, so
// the picture stays exactly where it is.
useProject.subscribe((s, previous) => {
  if (s.bed.width !== previous.bed.width || s.bed.height !== previous.bed.height) void useCamera.getState().redraw();
  // Material moved or resized: what's on the bed has probably changed. Not
  // when the move is to an outline found in this very picture.
  const changed =
    s.stockWidth !== previous.stockWidth ||
    s.stockLength !== previous.stockLength ||
    s.stockLeft !== previous.stockLeft ||
    s.stockFront !== previous.stockFront;
  if (changed && !applying && useCamera.getState().still) useCamera.setState({ stale: true });
  // Moved by typing, or by the assistant: the job goes somewhere new, so
  // remind the person to frame it before cutting.
  const moved = s.stockLeft !== previous.stockLeft || s.stockFront !== previous.stockFront;
  if (moved && !applying) useCamera.setState({ framePrompt: "moved" });
});

// After a job, the bed looks different from the picture.
useMachine.subscribe((s, previous) => {
  if (previous.job?.state === "running" && s.job && s.job.state !== "running" && useCamera.getState().still) {
    useCamera.setState({ stale: true });
  }
});
