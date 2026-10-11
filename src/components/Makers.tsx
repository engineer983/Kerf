import { useEffect, useState, type ReactNode } from "react";
import { X } from "lucide-react";
import { DEFAULT_BOX, DEFAULT_LAYERS, insideOf, type BoxInput, type LayersInput } from "../lib/design";
import { useProject } from "../state/project";
import { useToolbox } from "../state/tools";
import { useUi } from "../state/ui";
import { Checkbox, NumberField, Select } from "./controls";

const primary = "rounded-lg bg-accent px-4 py-2 font-medium text-accent-fg hover:opacity-90 disabled:opacity-40";
const quiet = "rounded-lg border border-line px-4 py-2 transition-colors hover:text-fg";

/** The box maker and the layers maker, whichever is open. */
export function Makers() {
  const maker = useUi((s) => s.maker);
  if (maker === "box") return <BoxMaker />;
  if (maker === "layers") return <LayerMaker />;
  return null;
}

interface MakerProps {
  title: string;
  about: string;
  action: string;
  /** Does the making; what it throws is shown as the reason it could not. */
  make: () => Promise<void>;
  children: ReactNode;
}

function Maker({ title, about, action, make, children }: MakerProps) {
  const close = () => useUi.getState().setMaker(null);
  const [working, setWorking] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && close();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  async function run() {
    setWorking(true);
    setProblem(null);
    try {
      await make();
      close();
    } catch (e) {
      setProblem(e instanceof Error ? e.message : String(e));
    } finally {
      setWorking(false);
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-8" onClick={close}>
      <div
        role="dialog"
        aria-label={title}
        className="flex max-h-full w-full max-w-[440px] flex-col overflow-hidden rounded-2xl border border-line bg-surface shadow-xl"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="flex items-center border-b border-line px-5 py-3">
          <h2 className="text-[14px] font-semibold">{title}</h2>
          <button onClick={close} aria-label={`Close ${title.toLowerCase()}`} className="ml-auto rounded p-1 text-muted hover:text-fg">
            <X size={16} />
          </button>
        </header>
        <div className="flex min-h-0 flex-col gap-3 overflow-y-auto p-5">
          <p className="text-muted">{about}</p>
          {children}
          {problem && (
            <p role="alert" className="rounded-lg border border-accent/40 px-3 py-2 text-accent">
              {problem}
            </p>
          )}
        </div>
        <footer className="flex justify-end gap-2 border-t border-line px-5 py-3">
          <button onClick={close} title="Close without making anything" className={quiet}>
            Cancel
          </button>
          <button onClick={() => void run()} disabled={working} title="Draw it and set the job up to cut it" className={primary}>
            {working ? "Working…" : action}
          </button>
        </footer>
      </div>
    </div>
  );
}

function BoxMaker() {
  const machineKind = useProject((s) => s.machineKind);
  const design = useProject((s) => s.design);
  const thickness = useProject((s) => s.thickness);
  const flatBits = useToolbox((s) => s.tools).filter((t) => t.kind === "flat");
  // Changing a box starts from what it was made with.
  const [box, setBox] = useState<BoxInput>(() =>
    design?.kind === "box" ? design.input : { ...DEFAULT_BOX, thickness, toolId: flatBits[0]?.id ?? null },
  );
  const change = (patch: Partial<BoxInput>) => setBox((b) => ({ ...b, ...patch }));
  const [w, d, h] = insideOf(box).map((mm) => Number(mm.toFixed(1)));

  return (
    <Maker
      title="Box maker"
      about="A box whose sides lock together with finger joints. Give the outside size and the thickness of your material; the panels are drawn to fit each other and laid out ready to cut."
      action={design?.kind === "box" ? "Make it again" : "Make the box"}
      make={() => useProject.getState().makeBox(box)}
    >
      <NumberField label="Width" unit="mm" min={10} max={3000} value={box.width} hint={{ text: "The box's outside size from left to right." }} onChange={(width) => change({ width })} />
      <NumberField label="Depth" unit="mm" min={10} max={3000} value={box.depth} hint={{ text: "The box's outside size from front to back." }} onChange={(depth) => change({ depth })} />
      <NumberField label="Height" unit="mm" min={10} max={3000} value={box.height} hint={{ text: "The box's outside height." }} onChange={(height) => change({ height })} />
      <NumberField
        label="Material thickness"
        unit="mm"
        min={0.5}
        max={50}
        value={box.thickness}
        hint={{ text: "Measure your sheet: the joints are cut exactly this deep, so a sheet sold as 3 mm that is really 2.7 mm will leave the fingers standing proud." }}
        onChange={(t) => change({ thickness: t })}
      />
      <Checkbox label="With a lid" hint={{ text: "Adds a top panel that joins on like the bottom, making a closed box. Leave one side unglued if you need to get in." }} checked={box.lid} onChange={(lid) => change({ lid })} />
      <NumberField
        label="Finger width"
        unit="mm"
        min={0}
        max={500}
        value={box.fingerWidth}
        hint={{ text: "Roughly how wide each finger of a joint is. 0 chooses about three times the material's thickness. Each edge always gets a whole, odd number of fingers." }}
        onChange={(fingerWidth) => change({ fingerWidth })}
      />
      {machineKind === "laser" ? (
        <NumberField
          label="Kerf"
          unit="mm"
          step={0.01}
          min={0}
          max={2}
          value={box.kerf}
          hint={{ text: "The width of the line your laser burns away, usually 0.1 to 0.2 mm. The panels are drawn larger by half of it all round so the joints come out snug. If the joints are loose, raise it a little; if they will not go together, lower it." }}
          onChange={(kerf) => change({ kerf })}
        />
      ) : flatBits.length > 0 ? (
        <Select
          label="Bit"
          hint={{ text: "The flat end mill the panels will be cut with. The cut is moved out by half its width, and carried into each inside corner, since a round bit cannot cut a square one." }}
          value={box.toolId ?? flatBits[0].id}
          options={flatBits.map((t) => ({ value: t.id, label: t.name }))}
          onChange={(toolId) => change({ toolId })}
        />
      ) : (
        <p role="alert" className="text-accent">
          The toolbox has no flat end mill to cut the panels with. Add one first.
        </p>
      )}
      <p className="text-muted">
        Inside it will be {w} × {d} × {h} mm.
        {machineKind === "cnc" &&
          " Nothing holds a panel once it is cut free, so stick the sheet down with double-sided tape and put scrap board underneath."}
      </p>
    </Maker>
  );
}

function LayerMaker() {
  const machineKind = useProject((s) => s.machineKind);
  const design = useProject((s) => s.design);
  const widthMm = useProject((s) => s.widthMm);
  const photo = useProject((s) => s.image?.photo ?? false);
  const bed = useProject((s) => s.bed);
  const [layers, setLayers] = useState<LayersInput>(() =>
    design?.kind === "layers"
      ? design.input
      : // A photograph's brightness is not its shape, so its depth is estimated.
        { ...DEFAULT_LAYERS, widthMm: design ? DEFAULT_LAYERS.widthMm : widthMm, source: photo ? "depth" : "brightness" },
  );
  const change = (patch: Partial<LayersInput>) => setLayers((l) => ({ ...l, ...patch }));

  return (
    <Maker
      title="Stacked layers"
      about="Slices the picture into layers like the contour lines of a map. Cut them from thin sheet and glue each onto the one before: the picture's high points end up standing proud."
      action={design?.kind === "layers" ? "Make them again" : "Make the layers"}
      make={() => useProject.getState().makeLayers(layers)}
    >
      <NumberField
        label="Layers"
        step={1}
        min={2}
        max={12}
        value={layers.layers}
        hint={{ text: "How many layers to cut, counting the base. More layers give finer steps and take more material and gluing." }}
        onChange={(count) => change({ layers: Math.round(count) })}
      />
      <NumberField
        label="Finished width"
        unit="mm"
        min={10}
        max={bed.width}
        value={layers.widthMm}
        hint={{ text: "How wide the finished piece is. Every layer is this size, and its height follows the picture's shape." }}
        onChange={(width) => change({ widthMm: width })}
      />
      <Select
        label="Heights from"
        hint={{ text: "What decides which parts stand highest. Brightness suits drawings and height maps. Estimated depth works out how near each part of a photograph is, which takes a while the first time." }}
        value={layers.source}
        options={[
          { value: "brightness", label: "Brightness" },
          { value: "depth", label: "Estimated depth" },
        ]}
        onChange={(source) => change({ source })}
      />
      <Select
        label="Highest parts"
        hint={{ text: "Which end of the range ends up on top: the light (or near) parts, or the dark (or far) ones." }}
        value={layers.lightIsHigh ? "light" : "dark"}
        options={[
          { value: "light", label: layers.source === "depth" ? "Nearest" : "Lightest" },
          { value: "dark", label: layers.source === "depth" ? "Farthest" : "Darkest" },
        ]}
        onChange={(end) => change({ lightIsHigh: end === "light" })}
      />
      <NumberField
        label="Smallest piece"
        unit="mm"
        min={0}
        max={100}
        value={layers.smallestPieceMm}
        hint={{ text: "Pieces smaller than this across are left out, since they would be too small to handle and glue." }}
        onChange={(smallestPieceMm) => change({ smallestPieceMm })}
      />
      {machineKind === "laser" ? (
        <Checkbox
          label="Mark where the next layer goes"
          hint={{ text: "Lightly marks each layer with the outline of the one that sits on it, so you can see where to glue. The marks get their own operation, to run at low power." }}
          checked={layers.marks}
          onChange={(marks) => change({ marks })}
        />
      ) : (
        <p className="text-muted">
          On a router the cut follows the middle of each line, so every piece comes out smaller by half the
          bit's width all round. Use a small bit, and stick the sheet down so pieces cannot come loose.
        </p>
      )}
    </Maker>
  );
}
