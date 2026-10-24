import { useState } from "react";
import { Drill, Zap } from "lucide-react";
import { CUSTOM_PRESET, MACHINE_PRESETS } from "../lib/machines";
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
