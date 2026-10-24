import { create } from "zustand";
import {
  cameraBedPicture,
  cameraError,
  listCameras,
  setCameraStill,
  takeStill,
  type CameraFit,
  type CameraInfo,
} from "../lib/camera";
import { useMachine } from "./machine";
import { useProject } from "./project";

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
}

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
        set({ still: { url: URL.createObjectURL(blob), ...size, takenAt: new Date() }, stale: false, busy: false });
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
      set({ deviceId: null, fitDeviceId: null, fit: null, linedUpAt: null, still: null, bedPictureUrl: null, error: null, stale: false });
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
  };
});

// The bed size decides whether the fit still holds, and the picture's size.
// Changing the origin corner doesn't: the fit is in canvas millimetres, so
// the picture stays exactly where it is.
useProject.subscribe((s, previous) => {
  if (s.bed.width !== previous.bed.width || s.bed.height !== previous.bed.height) void useCamera.getState().redraw();
  // Material moved or resized: what's on the bed has probably changed.
  if (s.stockWidth !== previous.stockWidth || s.stockLength !== previous.stockLength) {
    if (useCamera.getState().still) useCamera.setState({ stale: true });
  }
});

// After a job, the bed looks different from the picture.
useMachine.subscribe((s, previous) => {
  if (previous.job?.state === "running" && s.job && s.job.state !== "running" && useCamera.getState().still) {
    useCamera.setState({ stale: true });
  }
});
