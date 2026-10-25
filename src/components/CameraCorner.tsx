import { Camera, Eye, EyeOff, TriangleAlert } from "lucide-react";
import { NOT_LINED_UP_MESSAGE, notLinedUp, useCamera } from "../state/camera";
import { useMachine } from "../state/machine";
import { useProject } from "../state/project";
import { useUi } from "../state/ui";
import { FindMaterialButton } from "./FoundMaterial";

/** Above this thickness the top of the material looks noticeably shifted in the picture. */
const THICK_MM = 3;

const time = (d: Date) => d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
const day = (iso: string) => new Date(iso).toLocaleDateString([], { day: "numeric", month: "short" });

/**
 * The "Take a picture of the bed" button, in the canvas's bottom-right
 * corner, with the show/hide and see-through controls once there is a picture.
 */
export function CameraCorner() {
  const still = useCamera((s) => s.still);
  const fit = useCamera((s) => s.fit);
  const fitDeviceId = useCamera((s) => s.fitDeviceId);
  const deviceId = useCamera((s) => s.deviceId);
  const show = useCamera((s) => s.show);
  const opacity = useCamera((s) => s.opacity);
  const busy = useCamera((s) => s.busy);
  const hasPicture = useCamera((s) => s.bedPictureUrl !== null);
  const bed = useProject((s) => s.bed);
  const camera = useCamera.getState();
  const lined = !notLinedUp({ fit, fitDeviceId, deviceId, still: null }, bed);

  // Before a camera is set up, the button leads to where it is set up.
  const label = lined ? "Take a picture of the bed" : "Set up a camera";
  const onPress = () => {
    if (lined || deviceId) void camera.capture();
    else useProject.getState().setStep("device");
  };

  return (
    <div className="absolute right-3 bottom-3 flex cursor-default items-center gap-2 rounded-xl border border-line bg-surface/95 px-2 py-1.5 shadow-sm backdrop-blur" onPointerDown={(e) => e.stopPropagation()} onDoubleClick={(e) => e.stopPropagation()}>
      {hasPicture && (
        <>
          <button
            className="rounded p-1 text-muted hover:text-fg"
            aria-label={show ? "Hide the camera picture" : "Show the camera picture"}
            title={show ? "Hide the camera picture" : "Show the camera picture"}
            onClick={() => camera.setShow(!show)}
          >
            {show ? <Eye size={15} /> : <EyeOff size={15} />}
          </button>
          {show && (
            <input
              type="range"
              aria-label="How see-through the camera picture is"
              title="How strongly the camera picture shows"
              min={0.1}
              max={1}
              step={0.05}
              value={opacity}
              onChange={(e) => camera.setOpacity(e.target.valueAsNumber)}
              className="w-20 accent-[var(--accent)]"
            />
          )}
        </>
      )}
      {still && !lined && fit === null && (
        <button className="rounded-lg px-2 py-1 text-muted hover:text-fg" onClick={() => camera.openLineUp(true)}>
          Line up
        </button>
      )}
      {(lined || deviceId) && <FindMaterialButton className="rounded-lg border border-line px-2.5 py-1 text-fg hover:border-muted disabled:opacity-50" />}
      <button
        className="flex items-center gap-1.5 rounded-lg bg-accent px-2.5 py-1 font-medium text-accent-fg transition-opacity hover:opacity-90 disabled:opacity-50"
        disabled={busy}
        onClick={onPress}
      >
        <Camera size={14} /> {busy ? "Taking a picture…" : label}
      </button>
    </div>
  );
}

/** What the person needs to know about the camera picture, under the bed size. */
export function CameraNotes({ thickness }: { thickness: number }) {
  const still = useCamera((s) => s.still);
  const fit = useCamera((s) => s.fit);
  const fitDeviceId = useCamera((s) => s.fitDeviceId);
  const deviceId = useCamera((s) => s.deviceId);
  const shown = useCamera((s) => s.show && s.bedPictureUrl !== null);
  const stale = useCamera((s) => s.stale);
  const error = useCamera((s) => s.error);
  const originMoved = useMachine((s) => s.originMoved);
  const linedUpAt = useCamera((s) => s.linedUpAt);
  const bed = useProject((s) => s.bed);
  if (!still && !error) return null;
  const reason = still ? notLinedUp({ fit, fitDeviceId, deviceId, still }, bed) : null;

  const notes: { text: string; warn?: boolean; help?: boolean }[] = [];
  if (error) notes.push({ text: error, warn: true });
  if (reason) notes.push({ text: NOT_LINED_UP_MESSAGE[reason] });
  if (shown) {
    notes.push({ text: `Camera picture, taken ${time(still!.takenAt)}${stale ? " · may be out of date" : ""}` });
    if (originMoved) {
      notes.push({ text: "The work origin has been moved, so the picture doesn't show where the job will go.", warn: true });
    }
    if (thickness > THICK_MM) {
      notes.push({ text: "Lined up at the bed surface: the top of thick material looks shifted, more so towards the edges.", help: true });
    }
    // Also covers machines without homing, whose 0,0 is wherever the head was
    // at power-on: nothing Kerf can see says the zero has moved.
    notes.push({
      text: `The picture assumes the job's 0,0 is the bed corner${linedUpAt ? ` (camera lined up ${day(linedUpAt)})` : ""}. Check the position with Frame before cutting.`,
    });
  }

  return (
    <div className="flex flex-col gap-1">
      {notes.map((n) => (
        <div
          key={n.text}
          role={n.warn ? "alert" : undefined}
          className={`flex items-start gap-1.5 self-start rounded-md border bg-surface/90 px-2 py-1 text-[11px] ${n.warn ? "border-accent/50 text-fg" : "border-line text-muted"} ${n.help ? "pointer-events-auto" : ""}`}
        >
          {n.warn && <TriangleAlert size={12} className="mt-px shrink-0 text-accent" />}
          <span>
            {n.text}
            {n.help && (
              <>
                {" "}
                <button className="underline underline-offset-2 hover:text-fg" onClick={() => useUi.getState().setHelp("camera")}>
                  How much?
                </button>
              </>
            )}
          </span>
        </div>
      ))}
    </div>
  );
}
