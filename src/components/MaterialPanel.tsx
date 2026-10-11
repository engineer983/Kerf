import { MATERIALS } from "../lib/materials";
import { useProject } from "../state/project";
import { NumberField, Section, Select } from "./controls";

/** The piece of material the job is made on: what it is, and how big. */
export function MaterialPanel() {
  const materialId = useProject((s) => s.materialId);
  const thickness = useProject((s) => s.thickness);
  const stockWidth = useProject((s) => s.stockWidth);
  const stockLength = useProject((s) => s.stockLength);
  const bed = useProject((s) => s.bed);
  const origin = useProject((s) => s.origin);
  const machineKind = useProject((s) => s.machineKind);
  const actions = useProject.getState();
  const tooBig = stockWidth > bed.width || stockLength > bed.height;

  return (
    <div className="flex flex-col gap-5">
      <p className="text-muted">
        Say what you are working on and how big the piece is. The design is then laid out on it, at its
        real size.
      </p>
      <Section title="Material">
        <Select
          label="Type"
          hint={{ text: "The material you are working on. It sets how the preview looks and, on a router, the cutting speeds." }}
          value={materialId}
          options={MATERIALS.map((m) => ({ value: m.id, label: m.label }))}
          onChange={actions.setMaterial}
        />
        <NumberField label="Thickness" unit="mm" min={0.1} max={200} value={thickness} onChange={actions.setThickness} />
        <NumberField
          label="Width"
          unit="mm"
          step={1}
          min={1}
          max={5000}
          value={stockWidth}
          onChange={(value) => actions.setStock({ stockWidth: value })}
        />
        <NumberField
          label="Length"
          unit="mm"
          step={1}
          min={1}
          max={5000}
          value={stockLength}
          onChange={(value) => actions.setStock({ stockLength: value })}
        />
        {tooBig && (
          <p role="alert" className="text-accent">
            That is larger than the machine's {bed.width} × {bed.height} mm bed, so only part of it can
            be reached.
          </p>
        )}
        <p className="text-muted">
          The material sits in the {origin === "backLeft" ? "back" : "front"}-left corner of the bed.
          {machineKind === "cnc"
            ? " Router speeds are worked out from the material and the bit."
            : " Laser speed and power are yours to set; the material only changes the preview."}
        </p>
      </Section>
      <button
        onClick={() => actions.setStep("design")}
        className="rounded-lg bg-accent px-3 py-2 font-medium text-accent-fg transition-opacity hover:opacity-90"
      >
        Continue
      </button>
    </div>
  );
}
