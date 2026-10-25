import { useEffect, useState } from "react";
import { Check } from "lucide-react";
import { useMachine } from "../state/machine";
import { useProject } from "../state/project";

/** How long a two-click button stays armed, waiting for the second click. */
const ARM_MS = 5000;
/** How far the bit may go down looking for the plate, in millimetres. */
const PROBE_TRAVEL = 10;

const quiet = "rounded-lg border border-line px-3 py-2 transition-colors hover:text-fg disabled:opacity-40";
const primary =
  "rounded-lg bg-accent px-3 py-2 font-medium text-accent-fg transition-opacity hover:opacity-90 disabled:opacity-40";

/** A button state that needs a second click within a few seconds. */
export function useArmed(): [boolean, (armed: boolean) => void] {
  const [armed, setArmed] = useState(false);
  useEffect(() => {
    if (!armed) return;
    const timer = setTimeout(() => setArmed(false), ARM_MS);
    return () => clearTimeout(timer);
  }, [armed]);
  return [armed, setArmed];
}

function Done({ done, children }: { done: boolean; children: React.ReactNode }) {
  return (
    <li className="flex gap-2">
      <span className={`mt-0.5 shrink-0 ${done ? "text-accent" : "invisible"}`}>
        <Check size={14} />
      </span>
      <span>{children}</span>
    </li>
  );
}

/**
 * Setting Z with a touch plate: the light test, then a two-click probe.
 * Allowed before a job and while one waits for a bit change.
 */
export function ProbeControls({ disabled }: { disabled: boolean }) {
  const m = useMachine();
  const plate = useProject((s) => s.touchPlate);
  const [armed, setArmed] = useArmed();
  const touching = m.pins.includes("P");
  const probing = m.probe?.state === "running";

  if (plate === null) {
    return (
      <p className="text-muted">
        To set Z with a touch plate, enter its thickness in the Device step, under Changing bits.
      </p>
    );
  }
  return (
    <div className="flex flex-col gap-2 rounded-lg border border-line p-3">
      <p className="font-medium">Set Z with the touch plate ({plate} mm)</p>
      {!m.probeTested ? (
        <>
          <p className="text-muted">
            First check the plate works: clip it to the bit, press Test, then touch the plate to the bit. The light
            below should turn on.
          </p>
          <div className="flex items-center gap-3">
            <button className={quiet} disabled={disabled} onClick={() => void m.act({ type: "probeTest" })}>
              Test the probe
            </button>
            <span className={touching ? "text-accent" : "text-muted"}>{touching ? "● Touching" : "○ Not touching"}</span>
          </div>
        </>
      ) : (
        <>
          <p className="text-muted">
            Put the plate under the bit and clip it on, then jog the tip to within about {PROBE_TRAVEL} mm of the plate.
            Without the plate the bit goes down {PROBE_TRAVEL} mm into whatever is below.
          </p>
          <p className={touching ? "text-accent" : "text-muted"}>{touching ? "● The probe reads as touching" : "○ Not touching"}</p>
          <button
            className={armed ? primary : quiet}
            disabled={disabled || probing || touching}
            onClick={() => {
              if (armed) void m.act({ type: "probeZ", plate, travel: PROBE_TRAVEL });
              setArmed(!armed);
            }}
          >
            {probing ? "Probing…" : armed ? "Click again: plate under the bit and clip on?" : "Probe Z with the touch plate"}
          </button>
        </>
      )}
      {m.probe?.state === "failed" && m.probe.message && <p className="text-accent">{m.probe.message}</p>}
      {m.probe?.state === "done" && <p className="text-muted">Z is set. Take the clip off the bit and move the plate away.</p>}
    </div>
  );
}

/** What to do while a job waits for the bit to be changed. */
export function ChangeCard() {
  const m = useMachine();
  const routerByHand = useProject((s) => s.routerByHand);
  const [cleared, setCleared] = useState(false);
  const [armed, setArmed] = useArmed();
  const parts = m.parts;
  // Each new change starts with the tick off.
  const part = parts?.part ?? 0;
  useEffect(() => setCleared(false), [part]);
  if (!parts) return null;
  // The next part's bit, as generated when the job started.
  const next = m.jobParts[parts.part]?.tool ?? null;
  const nextName = next ? `T${next.number}: ${next.name}` : `bit ${parts.part + 1}`;
  const motorsLetGo = m.settings[1] !== undefined && Number(m.settings[1]) !== 255;
  const ready = parts.zSet && (!parts.plateUsed || cleared);

  return (
    <div className="flex flex-col gap-3 rounded-xl border border-accent/40 p-4">
      <p className="font-medium">
        Part {parts.part} of {parts.of} is done. Change the bit.
      </p>
      <ol className="flex flex-col gap-2">
        <Done done={parts.spindleOff}>
          {routerByHand
            ? "Switch the router off at its own switch and unplug it."
            : parts.spindleOff
              ? "Kerf has switched the spindle off; wait until it stops turning."
              : "Waiting for the machine to confirm the spindle is off…"}
        </Done>
        <Done done={false}>
          Fit {nextName}. Hold the router, not the gantry, when loosening the collet.
        </Done>
        <Done done={parts.zSet}>
          Set Z again: probe it with the touch plate below, or lower the bit onto the material with the jog pad and press
          Zero Z here. Raise it more with the jog pad if you need room.
        </Done>
        {parts.plateUsed && (
          <li className="ml-6">
            <label className="flex items-center gap-2">
              <input type="checkbox" className="accent-accent" checked={cleared} onChange={(e) => setCleared(e.target.checked)} />
              Clip off the bit and plate moved away.
            </label>
          </li>
        )}
      </ol>
      {motorsLetGo && (
        <p className="text-muted">
          The motors let go while the machine waits, so the head can be pushed out of place. Change the bit gently, or
          set $1=255.
        </p>
      )}
      {/* Probing is over by the time a hand-switched router goes back on:
          a spinning bit is never lowered onto the plate. */}
      {routerByHand && ready ? (
        <p role="alert" className="rounded-lg border border-accent/40 px-3 py-2 text-accent">
          Z is set. Switch the router back on now, then Continue.
        </p>
      ) : (
        <ProbeControls disabled={m.state !== "idle"} />
      )}
      <div className="grid grid-cols-2 gap-2">
        <button
          className={primary}
          disabled={!ready || m.state !== "idle"}
          title={parts.zSet ? "Start the next part with the new bit; click again to confirm" : "Set Z again first"}
          onClick={() => {
            if (armed) void m.act({ type: "continueJob", plateCleared: cleared });
            setArmed(!armed);
          }}
        >
          {armed ? "New bit fitted and Z set?" : `Continue with bit ${parts.part + 1}`}
        </button>
        <button className={quiet} onClick={() => void m.act({ type: "stopJob" })}>
          Stop the job here
        </button>
      </div>
    </div>
  );
}
