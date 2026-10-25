import { useEffect, useRef, useState } from "react";
import { ArrowDown, ArrowLeft, ArrowRight, ArrowUp, RefreshCw } from "lucide-react";
import { useHeldJog, type JogDirection } from "../lib/heldJog";
import { inBrowser, isTauri } from "../lib/ipc";
import {
  JOG_SPEEDS,
  JOG_STEPS,
  jogFeed,
  storeJogSpeed,
  storeJogStep,
  storedJogSpeed,
  storedJogStep,
  type JogSpeed,
  type JogStep,
} from "../lib/jog";
import { MACHINE_PRESETS } from "../lib/machines";
import { jobIsActive, useMachine } from "../state/machine";
import { useProject } from "../state/project";
import { Section, Segmented } from "./controls";

const FRAME_FEED = 3000;
/** Going back to zero is one long move, so it never goes faster than framing. */
const GO_TO_ZERO_MAX_FEED = 3000;
/** Keys that move the machine, only while the jog pad has focus: the canvas
 * and text fields use the arrow keys too. */
const JOG_KEYS: Record<string, JogDirection> = {
  ArrowUp: [0, 1, 0],
  ArrowDown: [0, -1, 0],
  ArrowLeft: [-1, 0, 0],
  ArrowRight: [1, 0, 0],
  PageUp: [0, 0, 1],
  PageDown: [0, 0, -1],
};
/** How long the start button stays armed, waiting for the confirming click. */
const ARM_MS = 5000;

const quiet = "rounded-lg border border-line px-3 py-2 transition-colors hover:text-fg disabled:opacity-40";
const primary =
  "rounded-lg bg-accent px-3 py-2 font-medium text-accent-fg transition-opacity hover:opacity-90 disabled:opacity-40";

export function MachinePanel() {
  const preset = useProject((s) => MACHINE_PRESETS.find((p) => p.id === s.machinePreset));
  const program = useProject((s) => s.gcode.program);
  const machineKind = useProject((s) => s.machineKind);
  const m = useMachine();
  const [step, setStep] = useState<JogStep>(storedJogStep);
  const [speed, setSpeed] = useState<JogSpeed>(storedJogSpeed);
  const [command, setCommand] = useState("");
  const [armed, setArmed] = useState(false);
  const held = useHeldJog();
  const heldKey = useRef<string | null>(null);
  const [padFocused, setPadFocused] = useState(false);

  useEffect(() => {
    if (isTauri() && !inBrowser) void useMachine.getState().refreshPorts();
  }, []);

  useEffect(() => {
    if (!armed) return;
    const timer = setTimeout(() => setArmed(false), ARM_MS);
    return () => clearTimeout(timer);
  }, [armed]);

  if (!isTauri()) {
    return <p className="text-muted">Machine control needs the desktop app.</p>;
  }
  if (inBrowser) {
    return (
      <p className="text-muted">
        Kerf in a browser does not connect to a machine. Export the G-code from Operations and send it
        with your machine's own software, or use the desktop app, which can connect to a GRBL machine
        and run the job itself.
      </p>
    );
  }
  if (preset?.exportOnly) {
    return (
      <p className="text-muted">
        The {preset.label} is export-only: its firmware is not GRBL, so Kerf does not connect to it.
        Export the G-code from Operations and send it with the maker's software or LightBurn.
      </p>
    );
  }

  const running = jobIsActive(m.job);
  const idle = m.connected && m.state === "idle" && !running;
  // Moving, or perhaps about to: still working through a job's last lines,
  // jogging, paused, homing, or stopped by the safety door. Stop shows for all
  // of them; while homing it resets the machine at once.
  const moving =
    running || (m.state !== null && ["run", "jog", "hold", "home", "door"].includes(m.state));
  const canDisconnect = !running && (m.state === null || ["idle", "alarm", "sleep"].includes(m.state));
  const modeProblem = laserModeProblem(machineKind, m.settings[32]);
  const homingOff = m.settings[22] !== undefined && Number(m.settings[22]) === 0;
  const homingOn = m.settings[22] !== undefined && Number(m.settings[22]) === 1;
  const holding = step === "hold";
  const feedFor = ([, y, z]: JogDirection) => jogFeed(speed, z ? 2 : y ? 1 : 0, m.settings);
  // One step per press. Only offered when idle.
  const jogStep = (direction: JogDirection) => {
    if (!idle || holding) return;
    const [x, y, z] = direction.map((d) => d * Number(step));
    void m.act({ type: "jog", x, y, z, feed: feedFor(direction) });
  };
  const holdStart = (direction: JogDirection) => {
    if (idle && holding) held.start(direction, feedFor(direction));
  };
  // While a hold runs the state is "jog", but the buttons must stay live so
  // that letting go is seen.
  const padDisabled = !idle && !held.active;

  const onPadKey = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      if (held.active) held.stop();
      else if (m.state === "jog") void m.act({ type: "cancelJog" });
      return;
    }
    const direction = JOG_KEYS[e.key];
    if (!direction) return;
    // The arrow keys would otherwise scroll the panel.
    e.preventDefault();
    // Auto-repeat neither steps again nor restarts a hold; the hold's own
    // keep-alive runs until the key comes up.
    if (e.repeat) return;
    if (holding) {
      if (!held.active) heldKey.current = e.key;
      holdStart(direction);
    } else {
      jogStep(direction);
    }
  };
  const onPadKeyUp = (e: React.KeyboardEvent) => {
    if (e.key === heldKey.current) {
      heldKey.current = null;
      held.stop();
    }
  };
  const jogButton = (label: string, direction: JogDirection) => (
    <JogButton
      label={label}
      direction={direction}
      disabled={padDisabled}
      holding={holding}
      onStep={jogStep}
      onHoldStart={holdStart}
      onHoldEnd={held.stop}
    />
  );
  const goToZeroFeed = Math.min(jogFeed(speed, 0, m.settings), jogFeed(speed, 1, m.settings), GO_TO_ZERO_MAX_FEED);

  return (
    <div className="flex flex-col gap-5">
      <Section title="Connection">
        <div className="flex gap-2">
          <select
            aria-label="Serial port"
            className="min-w-0 flex-1 rounded-md border border-line bg-raised px-2 py-1.5"
            value={m.port ?? ""}
            disabled={m.connected || m.connecting}
            onChange={(e) => m.setPort(e.target.value)}
          >
            {m.ports.length === 0 && <option value="">No serial ports found</option>}
            {m.ports.map((p) => (
              <option key={p.name} value={p.name}>
                {p.description ? `${p.name} · ${p.description}` : p.name}
              </option>
            ))}
          </select>
          <button
            onClick={() => void m.refreshPorts()}
            disabled={m.connected}
            aria-label="Refresh port list"
            className={`${quiet} px-2 text-muted`}
          >
            <RefreshCw size={14} />
          </button>
        </div>
        {m.connected ? (
          <button
            onClick={() => void m.disconnect()}
            disabled={!canDisconnect}
            title={canDisconnect ? undefined : "Stop the machine before disconnecting"}
            className={`${quiet} text-muted`}
          >
            Disconnect
          </button>
        ) : (
          <button onClick={() => void m.connect()} disabled={!m.port || m.connecting} className={primary}>
            {m.connecting ? "Connecting…" : "Connect"}
          </button>
        )}
        {m.connected && (
          <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5">
            <dt className="text-muted">State</dt>
            <dd className="capitalize">{m.state ?? "No reply yet"}</dd>
            <dt className="text-muted">From work zero</dt>
            <dd className="font-mono text-[12px]">{formatPosition(m.position)}</dd>
            {/* Without homing ($22=0) the machine's own position means nothing. */}
            {!homingOff && (
              <>
                <dt className="text-muted">From home</dt>
                <dd className="font-mono text-[12px]">
                  {formatPosition(m.machinePosition)}
                  {homingOn && <span className="ml-2 font-sans text-muted">(after homing)</span>}
                </dd>
              </>
            )}
            {m.version && (
              <>
                <dt className="text-muted">Firmware</dt>
                <dd>Grbl {m.version}</dd>
              </>
            )}
            {m.settings[130] && m.settings[131] && (
              <>
                <dt className="text-muted">Travel</dt>
                <dd className="font-mono text-[12px]">
                  {Number(m.settings[130]).toFixed(0)} × {Number(m.settings[131]).toFixed(0)} mm
                </dd>
              </>
            )}
          </dl>
        )}
        {m.error && (
          <p role="alert" className="rounded-lg border border-accent/40 px-3 py-2 text-accent">
            {m.error}
          </p>
        )}
      </Section>

      {m.connected && (
        <>
          <Section title="Move">
            <Segmented
              label="Jog distance in millimetres"
              value={step}
              options={JOG_STEPS}
              onChange={(value) => {
                setStep(value);
                storeJogStep(value);
              }}
            />
            <Segmented
              label="Jog speed"
              value={speed}
              options={JOG_SPEEDS}
              onChange={(value) => {
                setSpeed(value);
                storeJogSpeed(value);
              }}
            />
            {/* Keys are handled here only, never on the window. */}
            <div
              tabIndex={0}
              role="group"
              aria-label="Jog pad: arrow keys move X and Y, Page Up and Page Down move Z"
              title={holding ? "Hold a button or an arrow key to move; let go to stop" : undefined}
              onKeyDown={onPadKey}
              onKeyUp={onPadKeyUp}
              onFocus={() => setPadFocused(true)}
              onBlur={(e) => {
                if (e.currentTarget.contains(e.relatedTarget as Node | null)) return;
                setPadFocused(false);
                heldKey.current = null;
                held.stop();
              }}
              className="rounded-lg p-1 outline-none focus-within:ring-2 focus-within:ring-accent"
            >
              <div className="flex items-center justify-between">
                <div className="grid grid-cols-3 gap-1">
                  <span />
                  {jogButton("Jog back (Y+)", [0, 1, 0])}
                  <span />
                  {jogButton("Jog left (X−)", [-1, 0, 0])}
                  <span />
                  {jogButton("Jog right (X+)", [1, 0, 0])}
                  <span />
                  {jogButton("Jog forward (Y−)", [0, -1, 0])}
                  <span />
                </div>
                <div className="grid gap-1">
                  {jogButton("Raise (Z+)", [0, 0, 1])}
                  {jogButton("Lower (Z−)", [0, 0, -1])}
                </div>
              </div>
              <p className={`mt-1 text-[11px] text-accent ${padFocused ? "" : "invisible"}`}>Keys move the machine</p>
            </div>
            {m.state === "alarm" && (
              <p className="text-muted">
                {homingOff
                  ? "The machine is locked. Homing is switched off, so Unlock it once you know where the head is."
                  : "The machine is locked. Home it so it knows where it is (or Unlock if it has no homing switches and you know where it is)."}
              </p>
            )}
            <div className="grid grid-cols-2 gap-2">
              <button
                className={quiet}
                disabled={!(idle || (m.state === "alarm" && !running)) || homingOff}
                title={
                  homingOff
                    ? "Homing is switched off on this machine ($22=0)."
                    : machineKind === "cnc"
                      ? "Send the machine to its home switches to find its position. Raise the bit first: homing moves every axis, and after a stopped cut the bit may still be in the material"
                      : "Send the machine to its home switches to find its position"
                }
                onClick={() => void m.act({ type: "home" })}
              >
                Home
              </button>
              <button className={quiet} disabled={m.state !== "alarm"} onClick={() => void m.act({ type: "unlock" })}>Unlock</button>
              <button className={quiet} disabled={!idle} onClick={() => void m.act({ type: "setOrigin", z: false })}>Zero XY here</button>
              <button className={quiet} disabled={!idle} onClick={() => void m.act({ type: "setOrigin", z: true })}>Zero Z here</button>
              <button
                className={`${quiet} col-span-2`}
                disabled={!idle}
                onClick={() => void m.act({ type: "goToXyZero", feed: goToZeroFeed })}
              >
                Go to XY zero
              </button>
            </div>
          </Section>

          <Section title="Job">
            {!program && <p className="text-muted">Generate the G-code in Operations first.</p>}
            {program && !running && (
              <>
                <p className="text-muted">
                  Coordinates are measured from the work zero. Framing moves at the current height, so
                  raise the bit clear of the material and clamps first.
                </p>
                <button
                  className={quiet}
                  disabled={!idle}
                  onClick={() => void m.act({ type: "frame", ...program.bounds, feed: FRAME_FEED })}
                >
                  Frame the job outline
                </button>
                {modeProblem && (
                  <p role="alert" className="rounded-lg border border-accent/40 px-3 py-2 text-accent">
                    {modeProblem}
                  </p>
                )}
                <button
                  className={primary}
                  disabled={!idle || modeProblem !== null || program.programId === undefined}
                  onClick={() => {
                    if (armed && program.programId !== undefined) {
                      void m.act({ type: "startJob", programId: program.programId });
                    }
                    setArmed(!armed);
                  }}
                >
                  {armed ? "Click again to start the machine" : "Start job"}
                </button>
              </>
            )}
            {m.job && (
              <div>
                <div className="mb-1 flex justify-between">
                  <span className="capitalize">{m.job.state}</span>
                  <span className="font-mono text-[12px] text-muted">
                    {m.job.acknowledged.toLocaleString()} / {m.job.total.toLocaleString()}
                  </span>
                </div>
                <progress
                  className="h-1.5 w-full accent-accent"
                  max={m.job.total || 1}
                  value={m.job.acknowledged}
                />
                {m.job.message && <p className="mt-2 text-accent">{m.job.message}</p>}
              </div>
            )}
            {moving && (
              <div className="grid grid-cols-2 gap-2">
                {running && m.state === "hold" ? (
                  <button className={quiet} onClick={() => void m.act({ type: "resume" })}>Resume</button>
                ) : running ? (
                  <button className={quiet} onClick={() => void m.act({ type: "hold" })}>Pause</button>
                ) : (
                  <span />
                )}
                <button className={primary} onClick={() => void m.act({ type: "stopJob" })}>
                  Stop
                </button>
              </div>
            )}
          </Section>

          <Section title="Console">
            <pre className="h-36 overflow-auto rounded-lg border border-line bg-raised p-3 font-mono text-[11px] leading-relaxed whitespace-pre-wrap select-text">
              {m.console.join("\n")}
            </pre>
            <form
              onSubmit={(e) => {
                e.preventDefault();
                if (command.trim()) void m.act({ type: "line", text: command.trim() });
                setCommand("");
              }}
            >
              <input
                aria-label="Command to send"
                placeholder={running ? "Unavailable while a job runs" : "Send a command, e.g. $$"}
                disabled={running}
                className="w-full rounded-md border border-line bg-raised px-2 py-1.5 font-mono text-[12px] select-text"
                value={command}
                onChange={(e) => setCommand(e.target.value)}
              />
            </form>
          </Section>
        </>
      )}
    </div>
  );
}

function formatPosition(position: [number, number, number] | null): string {
  return position ? position.map((v) => v.toFixed(2)).join("  ") : "—";
}

/**
 * Why the machine's laser mode (GRBL setting $32) does not suit the job, or
 * null if it does. The desktop app refuses to start the job either way; this
 * says so before the button is pressed.
 */
function laserModeProblem(kind: "laser" | "cnc", value: string | undefined): string | null {
  if (value === undefined) {
    return "Kerf has not read the machine's settings yet, so it cannot check laser mode ($32). Send $$ from the console below.";
  }
  const on = Number(value) === 1;
  if (kind === "laser" && !on) {
    return "Laser mode is off ($32=" + value + "). Without it the laser stays on whenever the machine pauses. Send $32=1 from the console below.";
  }
  if (kind === "cnc" && on) {
    return "Laser mode is on ($32=1), so the spindle would stop on every rapid move. For a router, send $32=0 from the console below.";
  }
  return null;
}

interface JogButtonProps {
  label: string;
  direction: JogDirection;
  disabled: boolean;
  /** Move only while pressed, rather than one step per click. */
  holding: boolean;
  onStep: (direction: JogDirection) => void;
  onHoldStart: (direction: JogDirection) => void;
  onHoldEnd: () => void;
}

function JogButton({ label, direction, disabled, holding, onStep, onHoldStart, onHoldEnd }: JogButtonProps) {
  const [x, y, z] = direction;
  const icon =
    z > 0 ? "Z+" : z < 0 ? "Z−" : y > 0 ? <ArrowUp size={16} /> : y < 0 ? <ArrowDown size={16} /> : x < 0 ? <ArrowLeft size={16} /> : <ArrowRight size={16} />;
  return (
    <button
      aria-label={label}
      title={label}
      // Kept out of the tab order: the pad around the buttons is the one stop.
      tabIndex={-1}
      disabled={disabled}
      onClick={() => {
        if (!holding) onStep(direction);
      }}
      onPointerDown={(e) => {
        if (!holding || e.button !== 0) return;
        // Keeps the release coming here even if the pointer slides off.
        e.currentTarget.setPointerCapture(e.pointerId);
        onHoldStart(direction);
      }}
      onPointerUp={onHoldEnd}
      onPointerCancel={onHoldEnd}
      onLostPointerCapture={onHoldEnd}
      // A long touch would otherwise open a menu.
      onContextMenu={(e) => e.preventDefault()}
      className="flex h-9 w-9 touch-none items-center justify-center rounded-lg border border-line font-mono text-[12px] transition-colors select-none hover:text-accent disabled:opacity-40"
    >
      {icon}
    </button>
  );
}
