import { X } from "lucide-react";
import { isTauri, type OperationKind } from "../lib/ipc";
import { CUSTOM_PRESET, MACHINE_PRESETS } from "../lib/machines";
import { findMaterial, MATERIALS } from "../lib/materials";
import { TOOL_KIND_LABELS } from "../lib/tools";
import { artworkRect, stockRect, traceColors, useProject, type OperationItem } from "../state/project";
import { useToolbox } from "../state/tools";
import { NumberField, Section, Segmented, Select } from "./controls";

const ALL_SHAPES = "all";

function formatDuration(seconds: number): string {
  const total = Math.round(seconds);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  return h > 0 ? `${h} h ${m} min` : m > 0 ? `${m} min ${s} s` : `${s} s`;
}

const formatLength = (mm: number) =>
  mm >= 1000 ? `${(mm / 1000).toFixed(2)} m` : `${mm.toFixed(0)} mm`;

export function OperationsPanel() {
  const image = useProject((s) => s.image);
  const bed = useProject((s) => s.bed);
  const machineKind = useProject((s) => s.machineKind);
  const maxPowerS = useProject((s) => s.maxPowerS);
  const machinePreset = useProject((s) => s.machinePreset);
  const materialId = useProject((s) => s.materialId);
  const thickness = useProject((s) => s.thickness);
  const origin = useProject((s) => s.origin);
  const startGcode = useProject((s) => s.startGcode);
  const endGcode = useProject((s) => s.endGcode);
  const left = useProject((s) => s.left);
  const front = useProject((s) => s.front);
  const widthMm = useProject((s) => s.widthMm);
  const stockWidth = useProject((s) => s.stockWidth);
  const stockLength = useProject((s) => s.stockLength);
  const spindleRpm = useProject((s) => s.spindleRpm);
  const operations = useProject((s) => s.operations);
  const traced = useProject((s) => s.trace.result);
  const { program, busy, error, exportedTo } = useProject((s) => s.gcode);
  const actions = useProject.getState();

  if (!image) return <p className="text-muted">Import an image first.</p>;
  if (!isTauri()) {
    return <p className="text-muted">G-code needs the desktop app; it is not available in a browser.</p>;
  }

  const colors = traceColors(traced);
  const presetNote = MACHINE_PRESETS.find((p) => p.id === machinePreset)?.note;
  const art = artworkRect({ image, widthMm, left, front, bed, origin });
  const stock = stockRect({ stockWidth, stockLength, bed, origin });
  const overhangs =
    art !== null &&
    (art.x < stock.x - 0.01 ||
      art.y < stock.y - 0.01 ||
      art.x + art.width > stock.x + stock.width + 0.01 ||
      art.y + art.height > stock.y + stock.height + 0.01);

  return (
    <div className="flex flex-col gap-5">
      <Section title="Machine">
        <Select
          label="Preset"
          value={machinePreset}
          options={[
            { value: CUSTOM_PRESET, label: "Custom" },
            ...MACHINE_PRESETS.map((p) => ({ value: p.id, label: p.label })),
          ]}
          onChange={actions.applyMachinePreset}
        />
        {presetNote && <p className="text-muted">{presetNote}</p>}
        <Segmented
          label="Machine type"
          value={machineKind}
          options={[
            { value: "laser", label: "Laser" },
            { value: "cnc", label: "CNC router" },
          ]}
          onChange={actions.setMachineKind}
        />
        <NumberField
          label="Bed width"
          unit="mm"
          min={10}
          max={5000}
          value={bed.width}
          onChange={(width) => actions.setBed({ width })}
        />
        <NumberField
          label="Bed depth"
          unit="mm"
          min={10}
          max={5000}
          value={bed.height}
          onChange={(height) => actions.setBed({ height })}
        />
        <Select
          label="X0 Y0 corner"
          value={origin}
          options={[
            { value: "frontLeft", label: "Front left" },
            { value: "backLeft", label: "Back left" },
          ]}
          onChange={actions.setOrigin}
        />
        {(startGcode || endGcode) && (
          <p className="text-muted">
            Jobs start with <span className="font-mono text-[12px]">{startGcode.replace(/\n/g, " ")}</span>
            {endGcode && (
              <>
                {" "}and end with <span className="font-mono text-[12px]">{endGcode.replace(/\n/g, " ")}</span>
              </>
            )}
            .
          </p>
        )}
        {machineKind === "laser" && (
          <NumberField
            label="Full power (S)"
            min={1}
            max={100000}
            value={maxPowerS}
            onChange={actions.setMaxPowerS}
          />
        )}
        {machineKind === "cnc" && (
          <NumberField
            label="Router speed"
            unit="rpm"
            min={1000}
            max={60000}
            value={spindleRpm}
            onChange={actions.setSpindleRpm}
          />
        )}
      </Section>

      <Section title="Material">
        <Select
          label="Type"
          value={materialId}
          options={MATERIALS.map((m) => ({ value: m.id, label: m.label }))}
          onChange={actions.setMaterial}
        />
        <NumberField
          label="Thickness"
          unit="mm"
          min={0.1}
          max={200}
          value={thickness}
          onChange={actions.setThickness}
        />
        <NumberField
          label="Width"
          unit="mm"
          min={1}
          max={5000}
          value={stockWidth}
          onChange={(value) => actions.setStock({ stockWidth: value })}
        />
        <NumberField
          label="Length"
          unit="mm"
          min={1}
          max={5000}
          value={stockLength}
          onChange={(value) => actions.setStock({ stockLength: value })}
        />
        {overhangs && (
          <p role="alert" className="text-accent">
            The artwork reaches past the edge of the material.
          </p>
        )}
        <p className="text-muted">
          The material sits in the {origin === "backLeft" ? "back" : "front"}-left corner of the bed.
          {machineKind === "cnc"
            ? " Router speeds are worked out from the material and the bit."
            : " Laser speed and power are yours to set; the material only changes the preview."}
        </p>
      </Section>

      <Section title="Position">
        <NumberField
          label="From left"
          unit="mm"
          min={-5000}
          max={5000}
          value={left}
          onChange={(value) => actions.setPosition({ left: value })}
        />
        <NumberField
          label={origin === "backLeft" ? "From back" : "From front"}
          unit="mm"
          min={-5000}
          max={5000}
          value={front}
          onChange={(value) => actions.setPosition({ front: value })}
        />
      </Section>

      {operations.map((operation, index) => (
        <OperationCard
          key={operation.id}
          operation={operation}
          title={`Operation ${index + 1}`}
          colors={colors}
          removable={operations.length > 1}
        />
      ))}

      <button
        onClick={actions.addOperation}
        className="rounded-lg border border-line px-3 py-2 text-muted transition-colors hover:text-fg"
      >
        Add operation
      </button>

      <div className="flex flex-col gap-3">
        <button
          onClick={() => void actions.generateGcode()}
          disabled={busy || !traced}
          className="rounded-lg bg-accent px-3 py-2 font-medium text-accent-fg transition-opacity hover:opacity-90 disabled:opacity-40"
        >
          {busy ? "Generating…" : "Generate G-code"}
        </button>
        {error && (
          <p role="alert" className="rounded-lg border border-accent/40 px-3 py-2 text-accent">
            {error}
          </p>
        )}
        {program && (
          <>
            <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5">
              <dt className="text-muted">Estimated time</dt>
              <dd>{formatDuration(program.seconds)}</dd>
              <dt className="text-muted">Cutting</dt>
              <dd>{formatLength(program.cutMm)}</dd>
              <dt className="text-muted">Travel</dt>
              <dd>{formatLength(program.travelMm)}</dd>
              <dt className="text-muted">Lines</dt>
              <dd>{program.lineCount.toLocaleString()}</dd>
              <dt className="text-muted">Extent</dt>
              <dd className="font-mono text-[12px]">
                X {program.bounds.minX.toFixed(1)}–{program.bounds.maxX.toFixed(1)}
                <br />Y {program.bounds.minY.toFixed(1)}–{program.bounds.maxY.toFixed(1)}
              </dd>
            </dl>
            <pre className="max-h-40 overflow-auto rounded-lg border border-line bg-raised p-3 font-mono text-[11px] leading-relaxed select-text">
              {program.head}
            </pre>
            <button
              onClick={() => void actions.exportGcode()}
              className="rounded-lg border border-accent px-3 py-2 font-medium text-accent transition-opacity hover:opacity-80"
            >
              Export G-code…
            </button>
            {exportedTo && <p className="break-all text-muted select-text">Saved to {exportedTo}</p>}
          </>
        )}
      </div>
    </div>
  );
}

interface OperationCardProps {
  operation: OperationItem;
  title: string;
  colors: string[];
  removable: boolean;
}

function OperationCard({ operation, title, colors, removable }: OperationCardProps) {
  const { updateOperation, removeOperation, setOperationType, setOperationTool } =
    useProject.getState();
  const tools = useToolbox((s) => s.tools);
  const openToolbox = useToolbox((s) => s.setOpen);
  const material = useProject((s) => findMaterial(s.materialId));
  const { id, kind } = operation;
  const isCnc = kind.type === "cncProfile" || kind.type === "cncRelief";
  const setKind = (patch: Partial<OperationKind>) =>
    updateOperation(id, { kind: { ...kind, ...patch } as OperationKind });

  // A relief is generated for flat and ball-nose bits only.
  const usable = kind.type === "cncRelief" ? tools.filter((t) => t.kind !== "vbit") : tools;
  const tool = tools.find((t) => t.id === operation.toolId);

  return (
    <section className="flex flex-col gap-3 rounded-xl border border-line p-4">
      <div className="flex items-center">
        <h3 className="text-[11px] font-semibold uppercase tracking-wider text-muted">{title}</h3>
        {removable && (
          <button
            onClick={() => removeOperation(id)}
            aria-label={`Remove ${title.toLowerCase()}`}
            className="ml-auto rounded p-0.5 text-muted hover:text-fg"
          >
            <X size={14} />
          </button>
        )}
      </div>

      {isCnc ? (
        <Select
          label="Type"
          value={kind.type}
          options={[
            { value: "cncProfile", label: "Follow paths" },
            { value: "cncRelief", label: "3D relief" },
          ]}
          onChange={(type) => setOperationType(id, type)}
        />
      ) : (
        <Select
          label="Type"
          value={kind.type}
          options={[
            { value: "laserImage", label: "Image (photo)" },
            { value: "laserFill", label: "Fill (engrave)" },
            { value: "laserCut", label: "Line (cut)" },
          ]}
          onChange={(type) => setOperationType(id, type)}
        />
      )}

      {isCnc && (
        <>
          <Select
            label="Bit"
            value={operation.toolId ?? ""}
            options={[
              ...(tool ? [] : [{ value: "", label: "Choose a bit" }]),
              ...usable.map((t) => ({ value: t.id, label: t.name })),
            ]}
            onChange={(toolId) => toolId && setOperationTool(id, toolId)}
          />
          <p className="text-muted">
            {tool
              ? `${TOOL_KIND_LABELS[tool.kind]}, ${tool.diameter} mm, ${tool.flutes} flute${tool.flutes === 1 ? "" : "s"}. `
              : ""}
            {material.machining
              ? `The speeds below were worked out for this bit in ${material.label.toLowerCase()}. They are starting points: try them on scrap.`
              : `${material.label} is not a routing material, so the speeds below were not calculated.`}{" "}
            <button className="text-accent hover:underline" onClick={() => openToolbox(true)}>
              Open toolbox
            </button>
          </p>
        </>
      )}

      {colors.length > 1 && kind.type !== "cncRelief" && kind.type !== "laserImage" && (
        <Select
          label="Applies to"
          value={operation.color ?? ALL_SHAPES}
          options={[
            { value: ALL_SHAPES, label: "All shapes" },
            ...colors.map((color) => ({ value: color, label: color })),
          ]}
          onChange={(color) => updateOperation(id, { color: color === ALL_SHAPES ? null : color })}
        />
      )}
      <NumberField
        label="Speed"
        unit="mm/min"
        min={1}
        max={100000}
        value={operation.feed}
        onChange={(feed) => updateOperation(id, { feed })}
      />

      {kind.type === "cncProfile" || kind.type === "cncRelief" ? (
        <>
          {kind.type === "cncRelief" && (
            <>
              <NumberField label="Stepover" unit="mm" min={0.02} max={50} value={kind.stepover} onChange={(stepover) => setKind({ stepover })} />
              <Select
                label="Raised areas"
                value={kind.lightIsHigh ? "light" : "dark"}
                options={[
                  { value: "light", label: "Light parts" },
                  { value: "dark", label: "Dark parts" },
                ]}
                onChange={(side) => setKind({ lightIsHigh: side === "light" })}
              />
            </>
          )}
          <NumberField label="Depth" unit="mm" min={0.01} max={200} value={kind.depth} onChange={(depth) => setKind({ depth })} />
          <NumberField label="Depth per pass" unit="mm" min={0.01} max={200} value={kind.depthPerPass} onChange={(depthPerPass) => setKind({ depthPerPass })} />
          <NumberField label="Plunge speed" unit="mm/min" min={1} max={100000} value={kind.plungeFeed} onChange={(plungeFeed) => setKind({ plungeFeed })} />
          <NumberField label="Safe height" unit="mm" min={0.1} max={200} value={kind.safeZ} onChange={(safeZ) => setKind({ safeZ })} />
          {tool && kind.depth > tool.cutLength && (
            <p role="alert" className="text-accent">
              Depth is more than this bit's {tool.cutLength} mm cutting length.
            </p>
          )}
          <p className="text-muted">
            {kind.type === "cncRelief"
              ? "Brightness sets the height, from the material's top (Z0) down to Depth. A photograph's brightness is not its shape, so a photo carves as a rough impression; a depth map or greyscale artwork made for carving gives a true relief. Blur in Trace smooths the surface."
              : "The bit's centre follows the path; there is no tool-width offset yet."}
          </p>
        </>
      ) : (
        <>
          <NumberField label="Power" unit="%" min={0} max={100} value={operation.power} onChange={(power) => updateOperation(id, { power })} />
          <NumberField label="Passes" min={1} max={100} value={operation.passes} onChange={(passes) => updateOperation(id, { passes: Math.round(passes) })} />
          {(kind.type === "laserFill" || kind.type === "laserImage") && (
            <NumberField label="Line interval" unit="mm" min={0.02} max={10} value={kind.interval} onChange={(interval) => setKind({ interval })} />
          )}
          {kind.type === "laserImage" && (
            <>
              <Select
                label="Shading"
                value={kind.dither ? "dither" : "grey"}
                options={[
                  { value: "dither", label: "Dots (dithered)" },
                  { value: "grey", label: "Varying power" },
                ]}
                onChange={(mode) => setKind({ dither: mode === "dither" })}
              />
              <p className="text-muted">
                Engraves the picture itself, so photographs keep their shading. Brightness and
                contrast in Trace adjust it. Dots suit wood, which does not darken evenly with power.
              </p>
            </>
          )}
        </>
      )}
    </section>
  );
}
