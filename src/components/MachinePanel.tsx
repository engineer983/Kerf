import { useEffect, useState } from "react";
import { ArrowDown, ArrowLeft, ArrowRight, ArrowUp, RefreshCw } from "lucide-react";
import { inBrowser, isTauri } from "../lib/ipc";
import { MACHINE_PRESETS } from "../lib/machines";
import { jobIsActive, useMachine } from "../state/machine";
import { useProject } from "../state/project";
import { Section, Segmented } from "./controls";

const JOG_FEED = 1500;
const JOG_FEED_Z = 300;
const FRAME_FEED = 3000;
/** How long the start button stays armed, waiting for the confirming click. */
const ARM_MS = 5000;

const quiet = "rounded-lg border border-line px-3 py-2 transition-colors hover:text-fg disabled:opacity-40";
const primary =
  "rounded-lg bg-accent px-3 py-2 font-medium text-accent-fg transition-opacity hover:opacity-90 disabled:opacity-40";

export function MachinePanel() {
  const preset = useProject((s) => MACHINE_PRESETS.find((p) => p.id === s.machinePreset));
  const program = useProject((s) => s.gcode.program);
  const m = useMachine();
  const [step, setStep] = useState("1");
  const [command, setCommand] = useState("");
  const [armed, setArmed] = useState(false);

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
  const distance = Number(step);
  const jog = (x: number, y: number, z: number) =>
    void m.act({ type: "jog", x: x * distance, y: y * distance, z: z * distance, feed: z ? JOG_FEED_Z : JOG_FEED });

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
          <button onClick={() => void m.disconnect()} disabled={running} className={`${quiet} text-muted`}>
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
            <dt className="text-muted">Position</dt>
            <dd className="font-mono text-[12px]">
              {m.position ? m.position.map((v) => v.toFixed(2)).join("  ") : "—"}
            </dd>
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
              options={["0.1", "1", "10", "100"].map((v) => ({ value: v, label: v }))}
              onChange={setStep}
            />
            <div className="flex items-center justify-between">
              <div className="grid grid-cols-3 gap-1">
                <span />
                <JogButton label="Jog back (Y+)" disabled={!idle} onClick={() => jog(0, 1, 0)}><ArrowUp size={16} /></JogButton>
                <span />
                <JogButton label="Jog left (X−)" disabled={!idle} onClick={() => jog(-1, 0, 0)}><ArrowLeft size={16} /></JogButton>
                <span />
                <JogButton label="Jog right (X+)" disabled={!idle} onClick={() => jog(1, 0, 0)}><ArrowRight size={16} /></JogButton>
                <span />
                <JogButton label="Jog forward (Y−)" disabled={!idle} onClick={() => jog(0, -1, 0)}><ArrowDown size={16} /></JogButton>
                <span />
              </div>
              <div className="grid gap-1">
                <JogButton label="Raise (Z+)" disabled={!idle} onClick={() => jog(0, 0, 1)}>Z+</JogButton>
                <JogButton label="Lower (Z−)" disabled={!idle} onClick={() => jog(0, 0, -1)}>Z−</JogButton>
              </div>
            </div>
            <div className="grid grid-cols-2 gap-2">
              <button className={quiet} disabled={!idle} onClick={() => void m.act({ type: "home" })}>Home</button>
              <button className={quiet} disabled={m.state !== "alarm"} onClick={() => void m.act({ type: "unlock" })}>Unlock</button>
              <button className={quiet} disabled={!idle} onClick={() => void m.act({ type: "setOrigin", z: false })}>Zero XY here</button>
              <button className={quiet} disabled={!idle} onClick={() => void m.act({ type: "setOrigin", z: true })}>Zero Z here</button>
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
                <button
                  className={primary}
                  disabled={!idle}
                  onClick={() => {
                    if (armed) void m.act({ type: "startJob" });
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
            {(running || m.state === "hold") && (
              <div className="grid grid-cols-2 gap-2">
                {m.state === "hold" ? (
                  <button className={quiet} onClick={() => void m.act({ type: "resume" })}>Resume</button>
                ) : (
                  <button className={quiet} onClick={() => void m.act({ type: "hold" })}>Pause</button>
                )}
                <button className={primary} onClick={() => void m.act({ type: running ? "stopJob" : "reset" })}>
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

interface JogButtonProps {
  label: string;
  disabled: boolean;
  onClick: () => void;
  children: React.ReactNode;
}

function JogButton({ label, disabled, onClick, children }: JogButtonProps) {
  return (
    <button
      aria-label={label}
      title={label}
      disabled={disabled}
      onClick={onClick}
      className="flex h-9 w-9 items-center justify-center rounded-lg border border-line font-mono text-[12px] transition-colors hover:text-accent disabled:opacity-40"
    >
      {children}
    </button>
  );
}
