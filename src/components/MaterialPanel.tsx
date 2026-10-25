import { MATERIALS } from "../lib/materials";
import { stockOffBed, useProject } from "../state/project";
import { NumberField, Section, Select } from "./controls";
import { FindMaterialButton, zeroWarning } from "./FoundMaterial";

const quiet = "rounded-lg border border-line px-3 py-1.5 transition-colors hover:text-accent disabled:opacity-40";

/** The piece of material the job is made on: what it is, how big, and where it sits. */
export function MaterialPanel() {
  const materialId = useProject((s) => s.materialId);
  const thickness = useProject((s) => s.thickness);
  const stockWidth = useProject((s) => s.stockWidth);
  const stockLength = useProject((s) => s.stockLength);
  const stockLeft = useProject((s) => s.stockLeft);
  const stockFront = useProject((s) => s.stockFront);
  const bed = useProject((s) => s.bed);
  const origin = useProject((s) => s.origin);
  const machineKind = useProject((s) => s.machineKind);
  const actions = useProject.getState();
  const tooBig = stockWidth > bed.width || stockLength > bed.height;
  const offBed = !tooBig && stockOffBed({ stockWidth, stockLength, stockLeft, stockFront, bed });
  const side = origin === "backLeft" ? "back" : "front";
  const inCorner = stockLeft === 0 && stockFront === 0;

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
          {machineKind === "cnc"
            ? "Router speeds are worked out from the material and the bit."
            : "Laser speed and power are yours to set; the material only changes the preview."}
        </p>
      </Section>
      <Section title="Where it sits">
        <NumberField
          label="From left"
          unit="mm"
          step={1}
          min={-5000}
          max={5000}
          value={stockLeft}
          hint={{ text: "How far the material's left edge is from the left edge of the bed. The design moves with it." }}
          onChange={(value) => actions.setStockPlace({ stockLeft: value })}
        />
        <NumberField
          label={origin === "backLeft" ? "From back" : "From front"}
          unit="mm"
          step={1}
          min={-5000}
          max={5000}
          value={stockFront}
          hint={{ text: `How far the material's ${side} edge is from the ${side} edge of the bed. The design moves with it.` }}
          onChange={(value) => actions.setStockPlace({ stockFront: value })}
        />
        <p className="text-muted">
          {inCorner
            ? `The material sits in the ${side}-left corner of the bed.`
            : `The material is ${stockLeft} mm from the left and ${stockFront} mm from the ${side}.`}
        </p>
        {!inCorner && (
          <p role="alert" className="text-accent">
            {zeroWarning({ origin, stockLeft, stockFront })}
          </p>
        )}
        {offBed && (
          <p role="alert" className="text-accent">
            Part of the material is off the {bed.width} × {bed.height} mm bed, so the machine can't reach all of it.
          </p>
        )}
        <div className="flex flex-wrap gap-2">
          <button
            className={quiet}
            disabled={inCorner}
            onClick={() => actions.setStockPlace({ stockLeft: 0, stockFront: 0 })}
            title={`Move the material, and the design on it, back into the ${side}-left corner`}
          >
            Put it back in the corner
          </button>
          <FindMaterialButton className={quiet} showReason />
        </div>
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
