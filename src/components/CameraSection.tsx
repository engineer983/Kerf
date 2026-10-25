import { Camera, TriangleAlert } from "lucide-react";
import { NOT_LINED_UP_MESSAGE, notLinedUp, useCamera } from "../state/camera";
import { useProject } from "../state/project";
import { Section, Select } from "./controls";

/** Choosing a camera over the bed and lining it up, in the Device step. */
export function CameraSection() {
  const cameras = useCamera((s) => s.cameras);
  const deviceId = useCamera((s) => s.deviceId);
  const still = useCamera((s) => s.still);
  const fit = useCamera((s) => s.fit);
  const fitDeviceId = useCamera((s) => s.fitDeviceId);
  const busy = useCamera((s) => s.busy);
  const error = useCamera((s) => s.error);
  const bed = useProject((s) => s.bed);
  const camera = useCamera.getState();
  const reason = notLinedUp({ fit, fitDeviceId, deviceId, still }, bed);

  return (
    <Section title="Camera">
      <p className="text-muted">
        A camera over the bed lets you see the design on top of what's really there. It's optional. Kerf opens the camera only
        when you take a picture, and the picture stays on this computer.
      </p>

      {cameras.length > 0 ? (
        <Select
          label="Camera"
          hint={{ text: "The camera pointed at the bed. Kerf remembers it." }}
          value={deviceId ?? cameras[0].deviceId}
          options={cameras.map((c) => ({ value: c.deviceId, label: c.label }))}
          onChange={camera.chooseCamera}
        />
      ) : (
        <button
          className="flex items-center justify-center gap-2 rounded-lg border border-line px-3 py-2 hover:border-muted disabled:opacity-50"
          disabled={busy}
          onClick={() => void camera.findCameras()}
        >
          <Camera size={15} /> Find cameras
        </button>
      )}

      {cameras.length > 0 && (
        <div className="flex gap-2">
          <button
            className="flex flex-1 items-center justify-center gap-2 rounded-lg border border-line px-3 py-2 hover:border-muted disabled:opacity-50"
            disabled={busy}
            onClick={() => void camera.capture()}
          >
            <Camera size={15} /> Take a picture
          </button>
          <button
            className="flex-1 rounded-lg border border-line px-3 py-2 hover:border-muted disabled:opacity-50"
            disabled={busy || !still}
            title={still ? undefined : "Take a picture first"}
            onClick={() => camera.openLineUp(true)}
          >
            Line up the camera
          </button>
        </div>
      )}

      {still && (
        <img src={still.url} alt="The latest picture from the camera" className="w-full rounded-lg border border-line" />
      )}
      {still && reason && <p className="text-muted">{NOT_LINED_UP_MESSAGE[reason]}</p>}
      {fit && !reason && <p className="text-muted">Lined up. Take a picture any time to see the bed under the design.</p>}

      {error && (
        <p role="alert" className="flex gap-2 rounded-lg border border-accent/40 px-3 py-2">
          <TriangleAlert size={15} className="mt-0.5 shrink-0 text-accent" />
          {error}
        </p>
      )}

      {(fit || cameras.length > 0) && (
        <button className="self-start text-muted underline-offset-2 hover:text-fg hover:underline" onClick={camera.forget}>
          Forget camera
        </button>
      )}
    </Section>
  );
}
