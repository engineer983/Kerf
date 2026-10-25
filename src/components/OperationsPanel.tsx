import { X } from "lucide-react";
import { CUT_COLOR, isFixed, MARK_COLOR } from "../lib/design";
import { isTauri, type OperationKind } from "../lib/ipc";
import { findMaterial } from "../lib/materials";
import { TOOL_KIND_LABELS } from "../lib/tools";
import {
  bitsForJob,
  everythingRect,
  stockRect,
  traceColors,
  useProject,
  type OperationItem,
} from "../state/project";
import { useToolbox } from "../state/tools";
import { useUi } from "../state/ui";
import { Checkbox, NumberField, Segmented, Select, Slider } from "./controls";
import { DesignPanel } from "./DesignPanel";
import { SimpleSetup } from "./SimpleSetup";

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
  const materialId = useProject((s) => s.materialId);
  const thickness = useProject((s) => s.thickness);
  const origin = useProject((s) => s.origin);
  const left = useProject((s) => s.left);
  const front = useProject((s) => s.front);
  const widthMm = useProject((s) => s.widthMm);
  const heightMm = useProject((s) => s.heightMm);
  const items = useProject((s) => s.items);
  const stockWidth = useProject((s) => s.stockWidth);
  const stockLength = useProject((s) => s.stockLength);
  const stockLeft = useProject((s) => s.stockLeft);
  const stockFront = useProject((s) => s.stockFront);
  const operations = useProject((s) => s.operations);
  const toolChange = useProject((s) => s.toolChange);
  // The bit list follows the toolbox too: a bit's name or number may change.
  useToolbox((s) => s.tools);
  const mode = useProject((s) => s.mode);
  const shaping = useProject((s) => s.depth.status === "working");
  const traced = useProject((s) => s.trace.result);
  const designed = useProject((s) => s.design !== null);
  const design = useProject((s) => s.design);
  // A box or a stack of layers: cut right through, at the size it was drawn.
  const fixed = useProject((s) => s.design !== null && isFixed(s.design));
  const { program, busy, error, exportedTo } = useProject((s) => s.gcode);
  const actions = useProject.getState();

  if (!image) return <p className="text-muted">Put something on the material first, in step 3, Design.</p>;
  if (!isTauri()) {
    return <p className="text-muted">G-code needs the desktop app; it is not available in a browser.</p>;
  }

  const colors = traceColors(traced);
  const bits = bitsForJob({ machineKind, operations, items, toolChange, thickness, design });
  const titleOf = (index: number) => {
    const op = operations[index];
    return op.forItems ? "Added text and drawings" : (fixed && op.color && LINE_NAMES[op.color]) || `Operation ${index + 1}`;
  };
  const art = everythingRect({ image, widthMm, heightMm, left, front, bed, origin, items });
  const stock = stockRect({ stockWidth, stockLength, stockLeft, stockFront, bed, origin });
  const overhangs =
    art !== null &&
    (art.x < stock.x - 0.01 ||
      art.y < stock.y - 0.01 ||
      art.x + art.width > stock.x + stock.width + 0.01 ||
      art.y + art.height > stock.y + stock.height + 0.01);

  return (
    <div className="flex flex-col gap-5">
      <div className="flex flex-col gap-2">
        <Segmented
          label="How much to show"
          value={mode}
          options={[
            { value: "simple", label: "Simple" },
            { value: "advanced", label: "Advanced" },
          ]}
          onChange={actions.setMode}
        />
        <p className="text-muted">
          {mode === "simple"
            ? "Say what you want and the settings are chosen for you."
            : "Every setting is yours to change. Switching back to Simple replaces them with the automatic setup."}
        </p>
      </div>
      {designed && <DesignPanel />}
      {!fixed && mode === "simple" && <SimpleSetup />}
      <p className="text-muted">
        {findMaterial(materialId).label}, {thickness} mm thick, {stockWidth} × {stockLength} mm. Change the
        material in step 2, and arrange the design in step 3.
      </p>
      {overhangs && (
        <p role="alert" className="text-accent">
          Something reaches past the edge of the material. Move or resize it in step 3, Design.
        </p>
      )}
      {(mode === "advanced" || fixed) &&
        operations.map((operation, index) => (
          <OperationCard
            key={operation.id}
            operation={operation}
            title={operation.forItems ? "Added text and drawings" : (fixed && operation.color && LINE_NAMES[operation.color]) || `Operation ${index + 1}`}
            colors={colors}
            removable={operations.length > 1 && !operation.forItems}
          />
        ))}

      {bits.runs.length > 0 && (
        <section className="flex flex-col gap-2 rounded-xl border border-line p-4">
          <h3 className="text-[11px] font-semibold uppercase tracking-wider text-muted">Bits for this job</h3>
          <ol className="flex flex-col gap-1">
            {bits.runs.map((run, i) => (
              <li key={i}>
                {i + 1}. {run.number !== null ? `T${run.number} ` : ""}
                {run.name}
                <span className="text-muted"> — {run.operations.map(titleOf).join(", ")}</span>
              </li>
            ))}
          </ol>
          <p className="text-muted">
            {bits.runs.length === 1
              ? "One bit, so the job runs without stopping."
              : toolChange.mode === "atc"
                ? `The automatic tool changer changes the bit ${bits.runs.length - 1} time${bits.runs.length === 2 ? "" : "s"}.`
                : `The job stops ${bits.runs.length - 1} time${bits.runs.length === 2 ? "" : "s"} to change the bit. Each time, the spindle is switched off and the bit raised, and you set Z again before it carries on.`}
          </p>
          {bits.reused && (
            <p className="text-accent">
              A bit is used again after another one, so you change bits more often than you need to. Where the order
              allows, put each bit's operations together.
            </p>
          )}
        </section>
      )}
      <div className={mode === "advanced" ? "grid grid-cols-2 gap-2" : "hidden"}>
        <button
          onClick={actions.addOperation}
          title="Add another operation to the job"
          className="rounded-lg border border-line px-3 py-2 text-muted transition-colors hover:text-fg"
        >
          Add operation
        </button>
        <button
          onClick={() => useUi.getState().setMaker("layers")}
          disabled={designed}
          title="Slice the picture into layers to cut from thin sheet and stack"
          className="col-span-2 rounded-lg border border-line px-3 py-2 text-muted transition-colors hover:text-fg disabled:opacity-40"
        >
          Stacked layers…
        </button>
        <button
          onClick={actions.addCutout}
          title="Add a cut around the artwork, right through the material, to free the piece"
          className="rounded-lg border border-line px-3 py-2 text-muted transition-colors hover:text-fg disabled:opacity-40"
        >
          Add cutout
        </button>
      </div>

      <div className="flex flex-col gap-3">
        <button
          onClick={() => void actions.generateGcode()}
          // Generating while the picture's shape is still being worked out
          // would quietly carve from brightness instead.
          disabled={busy || !traced || shaping}
          className="rounded-lg bg-accent px-3 py-2 font-medium text-accent-fg transition-opacity hover:opacity-90 disabled:opacity-40"
        >
          {busy ? "Generating…" : shaping ? "Working out the shape…" : "Generate G-code"}
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
            {program.parts.length > 1 ? (
              <>
                <p className="text-muted">
                  One file for each bit works with any sender. Fit that file's bit and set Z before running each one.
                </p>
                {program.parts.map((part, i) => (
                  <button
                    key={i}
                    onClick={() => void actions.exportGcode(i)}
                    title={i === 0 ? "Save the first bit's operations as a file of their own" : "Save this bit's operations as a file of their own; fit the bit and set Z before running it"}
                    className="rounded-lg border border-accent px-3 py-2 font-medium text-accent transition-opacity hover:opacity-80"
                  >
                    Save part {i + 1} ({part.tool?.name ?? "bit"})…
                  </button>
                ))}
                <button
                  onClick={() => void actions.exportGcode(undefined, true)}
                  className="rounded-lg border border-line px-3 py-2 text-muted transition-colors hover:text-fg"
                >
                  Save as one file with tool changes (M6)…
                </button>
                <p className="text-muted">
                  Plain GRBL refuses M6. Use this only with a sender that stops for the bit change and lets you set Z
                  again, such as bCNC, UGS, gSender or ioSender.
                </p>
              </>
            ) : (
              <button
                onClick={() => void actions.exportGcode()}
                className="rounded-lg border border-accent px-3 py-2 font-medium text-accent transition-opacity hover:opacity-80"
              >
                Export G-code…
              </button>
            )}
            {exportedTo && <p className="break-all text-muted select-text">Saved to {exportedTo}</p>}
          </>
        )}
      </div>
    </div>
  );
}

/** Where a relief's heights come from: the picture's brightness, or estimated depth. */
function HeightSource() {
  const heightSource = useProject((s) => s.heightSource);
  const depth = useProject((s) => s.depth);
  const detail = useProject((s) => s.detail);
  const depthFloor = useProject((s) => s.depthFloor);
  const { setHeightSource, setDetail, setDepthFloor, estimateDepth } = useProject.getState();

  return (
    <div className="flex flex-col gap-3 rounded-lg border border-line p-3">
      <Select
        label="Height from"
        value={heightSource}
        options={[
          { value: "brightness", label: "Brightness" },
          { value: "depth", label: "Estimated depth" },
        ]}
        onChange={setHeightSource}
      />
      {heightSource === "depth" && (
        <>
          {depth.status === "ready" && depth.url ? (
            <>
              <img src={depth.url} alt="Estimated depth: lighter is nearer" className="w-full rounded-md border border-line" />
              <Slider
                label="Picture detail"
                min={0}
                max={1}
                step={0.05}
                value={detail}
                percent
                onChange={setDetail}
              />
              <Slider
                label="Flatten background"
                min={0}
                max={0.9}
                step={0.05}
                value={depthFloor}
                percent
                onChange={setDepthFloor}
              />
            </>
          ) : (
            <button
              className="rounded-lg bg-accent px-3 py-2 font-medium text-accent-fg transition-opacity hover:opacity-90 disabled:opacity-50"
              disabled={depth.status === "working"}
              onClick={() => void estimateDepth()}
            >
              {depth.status === "working" ? (depth.message ?? "Working…") : "Estimate depth"}
            </button>
          )}
          {depth.status === "error" && (
            <p role="alert" className="text-accent">
              {depth.message}
            </p>
          )}
          <p className="text-muted">
            A model works out how near each part of the photo is, so faces and objects carve as
            shapes. It runs on this computer; the first use downloads it (about 27 MB) from Hugging
            Face. The photo is not sent anywhere.
          </p>
        </>
      )}
    </div>
  );
}

interface OperationCardProps {
  operation: OperationItem;
  title: string;
  colors: string[];
  removable: boolean;
}

/** What a design's two kinds of line are for, in place of their colours. */
const LINE_NAMES: Record<string, string> = { [CUT_COLOR]: "Lines to cut through", [MARK_COLOR]: "Guide marks" };

function OperationCard({ operation, title, colors, removable }: OperationCardProps) {
  const { updateOperation, removeOperation, setOperationType, setOperationTool } =
    useProject.getState();
  const tools = useToolbox((s) => s.tools);
  const openToolbox = useToolbox((s) => s.setOpen);
  const material = useProject((s) => findMaterial(s.materialId));
  const designed = useProject((s) => s.design !== null && isFixed(s.design));
  const { id, kind } = operation;
  const isCnc = kind.type === "cncProfile" || kind.type === "cncRelief" || kind.type === "cncCutout";
  const thickness = useProject((s) => s.thickness);
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
            { value: "cncProfile" as const, label: "Follow paths" },
            { value: "cncRelief" as const, label: "3D relief" },
            { value: "cncCutout" as const, label: "Cut out the piece" },
          ].filter((option) => !operation.forItems || option.value === "cncProfile")}
          hint={{ text: "What this operation does: follow the traced lines, carve the picture as a 3D relief, or cut around the artwork to free the piece." }}
          onChange={(type) => setOperationType(id, type)}
        />
      ) : (
        <Select
          label="Type"
          value={kind.type}
          options={[
            { value: "laserImage" as const, label: "Image (photo)" },
            { value: "laserFill" as const, label: "Fill (engrave)" },
            { value: "laserCut" as const, label: "Line (cut)" },
            { value: "laserCutout" as const, label: "Cut out the piece" },
          ].filter((option) => !operation.forItems || option.value === "laserFill" || option.value === "laserCut")}
          hint={{ text: "What this operation does: engrave the picture itself with its shading, fill traced shapes solidly, follow their outlines, or cut around the artwork to free the piece." }}
          onChange={(type) => setOperationType(id, type)}
        />
      )}

      <Checkbox
        label="Include in the job"
        checked={operation.enabled !== false}
        onChange={(enabled) => updateOperation(id, { enabled })}
      />

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

      {colors.length > 1 && (kind.type === "cncProfile" || kind.type === "laserFill" || kind.type === "laserCut") && (
        <Select
          label="Applies to"
          value={operation.color ?? ALL_SHAPES}
          options={[
            { value: ALL_SHAPES, label: "All shapes" },
            ...colors.map((color) => ({ value: color, label: (designed && LINE_NAMES[color]) || color })),
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

      {kind.type === "cncProfile" || kind.type === "cncRelief" || kind.type === "cncCutout" ? (
        <>
          {kind.type === "cncCutout" && (
            <>
              <NumberField label="Offset" unit="mm" min={-1000} max={1000} value={kind.offset} onChange={(offset) => setKind({ offset })} />
              <Select
                label="Cut side"
                value={kind.side}
                options={[
                  { value: "outside", label: "Outside the line" },
                  { value: "on", label: "On the line" },
                  { value: "inside", label: "Inside the line" },
                ]}
                onChange={(side) => setKind({ side })}
              />
              <Select
                label="Shape"
                value={kind.outline}
                options={[
                  { value: "rectangle", label: "Rectangle" },
                  { value: "ellipse", label: "Oval" },
                ]}
                onChange={(outline) => setKind({ outline })}
              />
              {kind.outline === "rectangle" && (
                <NumberField label="Corner radius" unit="mm" min={0} max={1000} value={kind.cornerRadius} onChange={(cornerRadius) => setKind({ cornerRadius })} />
              )}
              <NumberField label="Tabs" min={0} max={40} value={kind.tabs} onChange={(tabs) => setKind({ tabs: Math.round(tabs) })} />
              {kind.tabs > 0 && (
                <>
                  <NumberField label="Tab width" unit="mm" min={0.5} max={100} value={kind.tabWidth} onChange={(tabWidth) => setKind({ tabWidth })} />
                  <NumberField label="Tab height" unit="mm" min={0.1} max={50} value={kind.tabHeight} onChange={(tabHeight) => setKind({ tabHeight })} />
                </>
              )}
            </>
          )}
          {kind.type === "cncRelief" && (
            <>
              <HeightSource />
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
          {kind.type === "cncCutout" && Math.abs(kind.depth - thickness) > 0.001 && (
            <button className="self-start text-accent hover:underline" onClick={() => setKind({ depth: thickness })}>
              Set to the material's {thickness} mm thickness
            </button>
          )}
          <NumberField label="Depth per pass" unit="mm" min={0.01} max={200} value={kind.depthPerPass} onChange={(depthPerPass) => setKind({ depthPerPass })} />
          <NumberField label="Plunge speed" unit="mm/min" min={1} max={100000} value={kind.plungeFeed} onChange={(plungeFeed) => setKind({ plungeFeed })} />
          <NumberField label="Safe height" unit="mm" min={0.1} max={200} value={kind.safeZ} onChange={(safeZ) => setKind({ safeZ })} />
          {tool && kind.depth > tool.cutLength && (
            <p role="alert" className="text-accent">
              Depth is more than this bit's {tool.cutLength} mm cutting length.
            </p>
          )}
          <p className="text-muted">
            {kind.type === "cncCutout"
              ? "Cuts a line around the artwork, right through the material, to free the piece. Put scrap board underneath, since the bit goes through. Run it last, after any carving."
              : kind.type === "cncRelief"
              ? "Brightness sets the height, from the material's top (Z0) down to Depth. A photograph's brightness is not its shape, so a photo carves as a rough impression; a depth map or greyscale artwork made for carving gives a true relief. Blur in Trace smooths the surface."
              : "The bit's centre follows the path; there is no tool-width offset yet."}
          </p>
        </>
      ) : (
        <>
          <NumberField label="Power" unit="%" min={0} max={100} value={operation.power} onChange={(power) => updateOperation(id, { power })} />
          <NumberField label="Passes" min={1} max={100} value={operation.passes} onChange={(passes) => updateOperation(id, { passes: Math.round(passes) })} />
          {kind.type === "laserCutout" && (
            <>
              <NumberField label="Offset" unit="mm" min={-1000} max={1000} value={kind.offset} onChange={(offset) => setKind({ offset })} />
              <Select
                label="Shape"
                value={kind.outline}
                options={[
                  { value: "rectangle", label: "Rectangle" },
                  { value: "ellipse", label: "Oval" },
                ]}
                onChange={(outline) => setKind({ outline })}
              />
              {kind.outline === "rectangle" && (
                <NumberField label="Corner radius" unit="mm" min={0} max={1000} value={kind.cornerRadius} onChange={(cornerRadius) => setKind({ cornerRadius })} />
              )}
              <p className="text-muted">
                Cuts a line around the artwork, right through the material, to free the piece. It runs
                in the same job, after the engraving. The power, speed and passes that get through
                depend on your laser and material, so try them on an offcut; several faster passes
                often char less than one slow one.
              </p>
            </>
          )}
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
