import { useState } from "react";
import { Drill, Zap } from "lucide-react";
import { inBrowser } from "../lib/ipc";
import { CUSTOM_PRESET, MACHINE_PRESETS } from "../lib/machines";
import { useMachine } from "../state/machine";
import { setWarnOnBack, useProject, warnsOnBack } from "../state/project";
import { CameraSection } from "./CameraSection";
import { Checkbox, NumberField, Section, Select } from "./controls";

const card = (active: boolean) =>
  `flex w-full items-start gap-3 rounded-xl border p-3 text-left transition-colors ${
    active ? "border-accent bg-raised shadow-sm" : "border-line hover:border-muted"
  }`;

/** Choosing the machine, which decides what the rest of the app offers. */
export function DevicePanel() {
  const machinePreset = useProject((s) => s.machinePreset);
  const machineKind = useProject((s) => s.machineKind);
  const bed = useProject((s) => s.bed);
  const origin = useProject((s) => s.origin);
  const maxPowerS = useProject((s) => s.maxPowerS);
  const spindleRpm = useProject((s) => s.spindleRpm);
  const startGcode = useProject((s) => s.startGcode);
  const endGcode = useProject((s) => s.endGcode);
  const actions = useProject.getState();
  const preset = MACHINE_PRESETS.find((p) => p.id === machinePreset);
  const custom = machinePreset === CUSTOM_PRESET;
  const [warn, setWarn] = useState(warnsOnBack);

  return (
    <div className="flex flex-col gap-5">
      <Section title="Your machine">
        {MACHINE_PRESETS.map((p) => (
          <button key={p.id} className={card(p.id === machinePreset)} onClick={() => actions.applyMachinePreset(p.id)}>
            {p.kind === "laser" ? <Zap size={18} className="mt-0.5 shrink-0 text-accent" /> : <Drill size={18} className="mt-0.5 shrink-0 text-accent" />}
            <span>
              <span className="block font-medium">{p.label}</span>
              <span className="block text-muted">
                {p.kind === "laser" ? "Laser" : "CNC router"} · {p.bed.width} × {p.bed.height} mm
                {p.exportOnly ? " · G-code files only" : ""}
              </span>
            </span>
          </button>
        ))}
        <button className={card(custom && machineKind === "laser")} onClick={() => actions.setMachineKind("laser")}>
          <Zap size={18} className="mt-0.5 shrink-0 text-muted" />
          <span>
            <span className="block font-medium">Another laser</span>
            <span className="block text-muted">Any GRBL laser; set its size below</span>
          </span>
        </button>
        <button className={card(custom && machineKind === "cnc")} onClick={() => actions.setMachineKind("cnc")}>
          <Drill size={18} className="mt-0.5 shrink-0 text-muted" />
          <span>
            <span className="block font-medium">Another CNC router</span>
            <span className="block text-muted">Any GRBL router; set its size below</span>
          </span>
        </button>
        {preset?.note && <p className="text-muted">{preset.note}</p>}
      </Section>

      <Section title="Details">
        <NumberField label="Bed width" unit="mm" min={10} max={5000} value={bed.width} onChange={(width) => actions.setBed({ width })} />
        <NumberField label="Bed depth" unit="mm" min={10} max={5000} value={bed.height} onChange={(height) => actions.setBed({ height })} />
        <Select
          label="X0 Y0 corner"
          value={origin}
          options={[
            { value: "frontLeft", label: "Front left" },
            { value: "backLeft", label: "Back left" },
          ]}
          onChange={actions.setOrigin}
        />
        {machineKind === "laser" ? (
          <NumberField label="Full power (S)" min={1} max={100000} value={maxPowerS} onChange={actions.setMaxPowerS} />
        ) : (
          <NumberField label="Router speed" unit="rpm" min={1000} max={60000} value={spindleRpm} onChange={actions.setSpindleRpm} />
        )}
        {(startGcode || endGcode) && (
          <p className="text-muted">
            Jobs start with <span className="font-mono text-[12px]">{startGcode.replace(/\n/g, " ")}</span>
            {endGcode && (
              <>
                {" "}and end with <span className="font-mono text-[12px]">{endGcode}</span>
              </>
            )}
            .
          </p>
        )}
      </Section>

      {machineKind === "cnc" && <ChangingBits />}

      <CameraSection />

      <button
        onClick={() => actions.setStep("material")}
        className="rounded-lg bg-accent px-3 py-2 font-medium text-accent-fg transition-opacity hover:opacity-90"
      >
        Continue
      </button>

      <Section title="Preferences">
        <Checkbox
          label="Warn before clearing a generated job"
          hint={{ text: "Going back to an earlier step after generating a job clears that job, since it no longer matches. With this ticked, you are asked first." }}
          checked={warn}
          onChange={(on) => {
            setWarnOnBack(on);
            setWarn(on);
          }}
        />
      </Section>
    </div>
  );
}

/** How a router's bits are changed when a job uses more than one. */
function ChangingBits() {
  const toolChange = useProject((s) => s.toolChange);
  const touchPlate = useProject((s) => s.touchPlate);
  const routerByHand = useProject((s) => s.routerByHand);
  const { setToolChange, setTouchPlate, setRouterByHand } = useProject.getState();
  const machinePosition = useMachine((s) => (s.connected ? s.machinePosition : null));
  const homed = useMachine((s) => s.connected && s.homed);
  // The change place is where the bit goes up to, so it is only taken from
  // a homed machine with Z near the top, never from a bit down at the work.
  const captureProblem = !machinePosition
    ? "Connect to the machine and home it first"
    : !homed
      ? "Home the machine first: the place is measured from home"
      : machinePosition[2] < -5
        ? "Raise Z to within 5 mm of the top first: the bit goes up to this height at each change"
        : null;
  const place = toolChange.place;
  const fixedAt = place.type === "machine" ? place.position : null;
  const setFixed = (axis: number, value: number) => {
    const position = [...(fixedAt ?? [0, 0, 0])] as [number, number, number];
    position[axis] = value;
    setToolChange({ place: { type: "machine", position } });
  };
  const [plateText, setPlateText] = useState(touchPlate === null ? "" : String(touchPlate));

  return (
    <Section title="Changing bits">
      <p className="text-muted">
        For jobs that use more than one bit. The job stops at each change, with the spindle off and the bit raised, and
        carries on once you have fitted the next bit and set Z again.
      </p>
      <Select
        label="How bits are changed"
        value={toolChange.mode}
        options={[
          { value: "byHand", label: "By hand" },
          { value: "atc", label: "Automatic tool changer (grblHAL)" },
        ]}
        onChange={(mode) => setToolChange({ mode })}
      />
      {toolChange.mode === "atc" ? (
        <p className="text-muted">
          The controller changes the bit itself at each T and M6, so every bit needs a tool number in the toolbox.
          This has not been tried on a real changer: test it with nothing on the bed first.
        </p>
      ) : (
        <Select
          label="Where to change them"
          value={place.type}
          options={[
            { value: "overWorkZero", label: "Over the work zero" },
            { value: "machine", label: "A fixed place on the machine" },
          ]}
          onChange={(type) =>
            setToolChange({ place: type === "machine" ? { type, position: fixedAt ?? [0, 0, 0] } : { type } })
          }
        />
      )}
      {toolChange.mode === "byHand" && fixedAt && (
        <>
          <p className="text-muted">
            Measured from the machine's home, so this needs a machine that homes: Kerf won't start the job until it
            has homed. The bit goes straight up to this height first, then across.
          </p>
          <NumberField label="X from home" unit="mm" min={-5000} max={5000} value={fixedAt[0]} onChange={(v) => setFixed(0, v)} />
          <NumberField label="Y from home" unit="mm" min={-5000} max={5000} value={fixedAt[1]} onChange={(v) => setFixed(1, v)} />
          <NumberField label="Z from home" unit="mm" min={-500} max={0} value={fixedAt[2]} onChange={(v) => setFixed(2, v)} />
          {!inBrowser && (
            <button
              className="rounded-lg border border-line px-3 py-2 text-muted transition-colors hover:text-fg disabled:opacity-40"
              disabled={captureProblem !== null}
              title={captureProblem ?? undefined}
              onClick={() =>
                machinePosition &&
                captureProblem === null &&
                setToolChange({ place: { type: "machine", position: [...machinePosition] as [number, number, number] } })
              }
            >
              Use the current position
            </button>
          )}
          {!inBrowser && captureProblem && machinePosition && <p className="text-muted">{captureProblem}.</p>}
          <p className="text-muted">
            Kerf checks at the start, and after each change, that this height is well above the material. Choose a Z
            near the top of the machine.
          </p>
        </>
      )}
      <NumberField
        label="Raise for changing bits"
        unit="mm"
        min={1}
        max={200}
        value={toolChange.lift}
        hint={{ text: "How far above the material the bit is raised to change it. The job's highest safe height is used if that is more. At the change you can raise it more with the jog pad." }}
        onChange={(lift) => setToolChange({ lift })}
      />
      <NumberField
        label="Time for the spindle to stop"
        unit="s"
        min={1}
        max={120}
        step={1}
        value={toolChange.spinDown}
        onChange={(spinDown) => setToolChange({ spinDown })}
      />
      {inBrowser ? (
        <p className="text-muted">Touch plate and router switch settings are in the desktop app, which runs the machine.</p>
      ) : (
        <>
          <label className="flex items-center justify-between gap-3">
            <span>Touch plate thickness</span>
            <span className="flex items-center gap-1">
              <input
                aria-label="Touch plate thickness in millimetres"
                inputMode="decimal"
                placeholder="No plate"
                className="w-24 rounded-md border border-line bg-raised px-2 py-1 text-right select-text"
                value={plateText}
                onChange={(e) => setPlateText(e.target.value)}
                onBlur={() => {
                  const value = Number(plateText);
                  const ok = plateText.trim() !== "" && Number.isFinite(value) && value > 0 && value <= 50;
                  setTouchPlate(ok ? value : null);
                  setPlateText(ok ? String(value) : "");
                }}
              />
              <span className="text-muted">mm</span>
            </span>
          </label>
          <p className="text-muted">
            Measure your plate with calipers. It stays blank until you enter it, because a wrong thickness sets Z wrongly.
          </p>
          <Checkbox
            label="My router is switched on and off by hand"
            hint={{ text: "Trim routers on many X-Carves and Shapeokos ignore M3 and M5. Kerf then reminds you to switch the router off at each bit change, and back on before carrying on." }}
            checked={routerByHand}
            onChange={setRouterByHand}
          />
        </>
      )}
    </Section>
  );
}
