import { useEffect, useState, type ReactNode } from "react";
import { X } from "lucide-react";
import { DEFAULT_BOX, DEFAULT_LAYERS, DEFAULT_TEXT, insideOf, ringPath, type BoxInput, type LayersInput } from "../lib/design";
import { inBrowser, listFonts, type TextSpec } from "../lib/ipc";
import { useProject } from "../state/project";
import { useToolbox } from "../state/tools";
import { useUi } from "../state/ui";
import { BackgroundRemover } from "./BackgroundRemover";
import { Checkbox, NumberField, Select, Slider } from "./controls";

const primary = "rounded-lg bg-accent px-4 py-2 font-medium text-accent-fg hover:opacity-90 disabled:opacity-40";
const quiet = "rounded-lg border border-line px-4 py-2 transition-colors hover:text-fg";

/** Whichever of the makers is open: the box, the layers, text, or removing a background. */
export function Makers() {
  const maker = useUi((s) => s.maker);
  if (maker === "box") return <BoxMaker />;
  if (maker === "layers") return <LayerMaker />;
  if (maker === "text") return <TextMaker />;
  if (maker === "background") return <BackgroundRemover />;
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
        <div className="flex min-h-0 flex-col gap-3 overflow-y-auto p-5" onChangeCapture={() => setProblem(null)}>
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

/** Fonts tried first for new text, in order; the first one installed is used. */
const USUAL_FONTS = ["Arial", "Segoe UI", "Helvetica", "DejaVu Sans", "Liberation Sans"];
/** Lines of text are set this far apart, as a multiple of the font's size; the backend uses the same. */
const LINE_SPACING = 1.25;

function TextMaker() {
  const design = useProject((s) => s.design);
  const machineKind = useProject((s) => s.machineKind);
  const items = useProject((s) => s.items);
  // What is being made: a new item on the material, a change to one, or the main artwork.
  const target = useUi((s) => s.makerItem);
  const editing = typeof target === "number" ? items.find((item) => item.id === target) : undefined;
  const [fonts, setFonts] = useState<string[]>([]);
  const [text, setText] = useState<TextSpec>(
    () => editing?.text ?? (target === null && design?.kind === "text" ? design.input : DEFAULT_TEXT),
  );
  const change = (patch: Partial<TextSpec>) => setText((t) => ({ ...t, ...patch }));
  const bend = text.bend ?? 0;
  // Words can go round the main artwork when they are added beside it.
  const canGoRound = useProject((s) => s.image !== null) && target !== null;
  const layout = text.around && canGoRound ? "around" : bend !== 0 ? "curve" : "straight";
  const artworkWidth = useProject((s) => s.widthMm);
  const artworkHeight = useProject((s) => s.heightMm);
  const artworkSize = { width: artworkWidth, height: artworkHeight };

  // The fonts are this computer's own, so the list is asked for.
  useEffect(() => {
    listFonts()
      .then((names) => {
        setFonts(names);
        setText((t) => (t.font ? t : { ...t, font: USUAL_FONTS.find((f) => names.includes(f)) ?? names[0] ?? "" }));
      })
      .catch(() => setFonts([]));
  }, []);

  const adding = target === "new";
  const changing = editing !== undefined || (target === null && design?.kind === "text");
  const make = () => {
    const project = useProject.getState();
    // Whatever the layout is not, is left out, so a curve set earlier does not linger.
    const spec = { ...text, around: layout === "around" ? text.around : null, bend: layout === "curve" ? bend : 0 };
    if (adding) return project.addText(spec);
    if (editing) return project.changeText(editing.id, spec);
    return project.makeText(spec);
  };

  return (
    <Maker
      title={adding ? "Add text" : "Text"}
      about="Words in any font installed on this computer, drawn as the outlines of their letters, straight or bent along a curve. A laser can engrave them solid or trace round them; a router follows their edges."
      action={adding ? "Add the text" : changing ? "Make it again" : "Make the text"}
      make={make}
    >
      <label className="flex flex-col gap-1">
        Words
        <textarea
          autoFocus
          className="h-20 resize-none rounded-md border border-line bg-raised px-2 py-1.5 select-text"
          placeholder="Type what it should say. Press Enter for a new line."
          value={text.text}
          onChange={(e) => change({ text: e.target.value })}
        />
      </label>
      {fonts.length > 0 ? (
        <Select
          label="Font"
          hint={{ text: "The typeface. Bold, simple fonts engrave and carve most clearly; thin or script fonts can lose their fine strokes at small sizes." }}
          value={text.font}
          options={fonts.map((name) => ({ value: name, label: name }))}
          onChange={(font) => change({ font })}
        />
      ) : (
        <p className="text-muted">Looking for this computer's fonts…</p>
      )}
      {inBrowser && fonts.length > 0 && fonts.length <= 2 && (
        <p className="text-muted">
          Only the font that comes with Kerf is listed. To choose from this computer's fonts, use Chrome or
          Edge and allow this site to see them when asked (or under the padlock in the address bar), then
          open this again.
        </p>
      )}
      <NumberField
        label="Letter size"
        unit="mm"
        step={1}
        min={1}
        max={1000}
        value={text.sizeMm}
        hint={{ text: "The font's size. Capital letters stand about seven tenths of it, so 20 mm gives capitals about 14 mm tall. You can also resize the words on the canvas afterwards." }}
        onChange={(sizeMm) => change({ sizeMm })}
      />
      <Checkbox label="Bold" hint={{ text: "Uses the font's heavier weight, where it has one." }} checked={text.bold} onChange={(bold) => change({ bold })} />
      <Checkbox label="Italic" hint={{ text: "Uses the font's slanted style, where it has one." }} checked={text.italic} onChange={(italic) => change({ italic })} />
      <Select
        label="Layout"
        hint={{ text: "How the words run: in straight lines, bent along a curve, or round the artwork that is already on the material." }}
        value={layout}
        options={[
          { value: "straight" as const, label: "Straight" },
          { value: "curve" as const, label: "On a curve" },
          ...(canGoRound ? [{ value: "around" as const, label: "Round the artwork" }] : []),
        ]}
        onChange={(next) =>
          change(
            next === "straight"
              ? { bend: 0, around: null }
              : next === "curve"
                ? { bend: bend || 120, around: null }
                : { bend: 0, around: text.around ?? { shape: "circle", side: "top", gap: 5 } },
          )
        }
      />
      {layout === "straight" && (
        <Select
          label="Line up"
          hint={{ text: "How lines of different lengths are lined up with each other." }}
          value={text.align}
          options={[
            { value: "left", label: "Left" },
            { value: "center", label: "Centre" },
            { value: "right", label: "Right" },
          ]}
          onChange={(align) => change({ align })}
        />
      )}
      {layout === "curve" && (
        <Slider
          label="Curve"
          unit="°"
          min={-350}
          max={350}
          step={5}
          value={bend}
          hint={{ text: "Bends the words round a circle. To the right they arch over the top, to go above a picture; to the left they curve underneath, to go below one. 180 takes them half way round; 350 almost all the way. With several lines the circle is opened out until they all fit, so a tight curve bends them less than asked." }}
          onChange={(value) => change({ bend: value })}
        />
      )}
      {layout === "around" && text.around && (
        <>
          <Select
            label="Following"
            hint={{ text: "The line the words follow round the artwork: the circle through its corners, an oval that hugs a wide or tall picture more closely, or its own frame with the corners rounded." }}
            value={text.around.shape}
            options={[
              { value: "circle", label: "A circle" },
              { value: "oval", label: "An oval" },
              { value: "frame", label: "Its frame" },
            ]}
            onChange={(shape) => change({ around: { ...text.around!, shape } })}
          />
          <Select
            label="Placed"
            hint={{ text: "Where the words are centred. Long text carries on round the sides from there." }}
            value={text.around.side}
            options={[
              { value: "top", label: "Over the top" },
              { value: "bottom", label: "Under the bottom" },
            ]}
            onChange={(side) => change({ around: { ...text.around!, side } })}
          />
          <NumberField
            label="Gap"
            unit="mm"
            step={1}
            min={-100}
            max={500}
            value={text.around.gap}
            hint={{ text: "Clear space between the artwork and the words." }}
            onChange={(gap) => change({ around: { ...text.around!, gap } })}
          />
          <p className="text-muted">
            The words are drawn at the letter size above, to fit round the artwork as it is now. If you
            resize the artwork later, use Change the text to fit them round it again.
          </p>
        </>
      )}
      <TextPreview text={layout === "around" ? text : { ...text, around: null }} artwork={canGoRound ? artworkSize : null} />
      {machineKind === "cnc" && (
        <p className="text-muted">
          A router follows the edge of each letter with the middle of the bit, so letters come out a little
          heavier than drawn. Use a fine or V-shaped bit for small text.
        </p>
      )}
    </Maker>
  );
}

/** Roughly how the words will look, drawn by the browser in the same font and on the same curve. */
function TextPreview({ text, artwork }: { text: TextSpec; artwork: { width: number; height: number } | null }) {
  const style = {
    fontFamily: `"${text.font.replace(/"/g, "")}"`,
    fontWeight: text.bold ? 700 : 400,
    fontStyle: text.italic ? "italic" : "normal",
  };
  const box = "min-h-16 overflow-hidden rounded-lg border border-line bg-raised px-3 py-2";
  const lines = text.text.split("\n").filter((line) => line.trim());
  const bend = text.bend ?? 0;
  if (lines.length === 0) {
    return (
      <div aria-label="Preview" className={`${box} text-muted`}>
        A preview appears here.
      </div>
    );
  }
  if (text.around && artwork) {
    // The artwork as a plain block, with the words where they will sit round
    // it, at their true size: one unit to the millimetre.
    const { shape, side, gap } = text.around;
    const top = side === "top";
    const spacing = text.sizeMm * LINE_SPACING;
    const out = (i: number) => gap + (top ? (lines.length - 1 - i) * spacing : text.sizeMm * 0.75 + i * spacing);
    const reach = Math.hypot(artwork.width, artwork.height) / 2 + Math.max(0, gap) + (lines.length + 1) * spacing;
    return (
      <div aria-label="Preview" className={box}>
        <svg viewBox={`${-reach} ${-reach} ${reach * 2} ${reach * 2}`} className="mx-auto block max-h-64 w-full" fill="currentColor">
          <rect x={-artwork.width / 2} y={-artwork.height / 2} width={artwork.width} height={artwork.height} rx={2} opacity={0.18} />
          {lines.map((line, i) => (
            <g key={i}>
              <path id={`preview-ring-${i}`} d={ringPath(shape, side, artwork.width, artwork.height, out(i))} fill="none" />
              <text fontSize={text.sizeMm} textAnchor="middle" style={style}>
                <textPath href={`#preview-ring-${i}`} startOffset="50%">
                  {line.trim()}
                </textPath>
              </text>
            </g>
          ))}
        </svg>
      </div>
    );
  }
  if (bend === 0) {
    return (
      <div aria-label="Preview" className={`${box} text-[26px] leading-tight whitespace-pre-wrap`} style={{ ...style, textAlign: text.align }}>
        {text.text}
      </div>
    );
  }

  // The same circle the backend uses: the one on which the widest line takes
  // up the angle asked for. The drawing is made with a font 100 units high,
  // since browsers do not set type well at a size of one.
  const size = 100;
  const spacing = size * LINE_SPACING;
  const measure = document.createElement("canvas").getContext("2d")!;
  measure.font = `${style.fontStyle} ${style.fontWeight} ${size}px ${style.fontFamily}`;
  const widest = Math.max(...lines.map((line) => measure.measureText(line.trim()).width), size / 2);
  const over = bend > 0;
  const angle = (Math.min(350, Math.abs(bend)) * Math.PI) / 180;
  // Opened out, as the backend does, until every line has circle to sit on.
  const radius = Math.max(widest / angle, over ? (lines.length - 1) * spacing + size : size);
  const half = (175 * Math.PI) / 180;
  // The view is cropped to the part of the circle the words take up. Worked
  // out for words over the top, then turned over for words underneath.
  const outer = radius + (over ? size : (lines.length - 1) * spacing + size * 0.3);
  const inner = Math.max(0, radius - (over ? lines.length * spacing : size));
  const spread = angle / 2;
  const across = spread >= Math.PI / 2 ? outer : outer * Math.sin(spread) + size / 2;
  const lowest = -(spread >= Math.PI / 2 ? outer : inner) * Math.cos(spread) + size / 2;
  const [top, bottom] = over ? [-outer, lowest] : [-lowest, outer];
  return (
    <div aria-label="Preview" className={box}>
      <svg viewBox={`${-across} ${top} ${across * 2} ${bottom - top}`} className="mx-auto block max-h-56 w-full" fill="currentColor">
        {lines.map((line, i) => {
          const r = radius + i * (over ? -spacing : spacing);
          if (r <= size / 4) return null;
          const [dx, dy] = [r * Math.sin(half), r * Math.cos(half)];
          const y = over ? -dy : dy;
          return (
            <g key={i}>
              <path id={`preview-line-${i}`} d={`M${-dx} ${y}A${r} ${r} 0 1 ${over ? 1 : 0} ${dx} ${y}`} fill="none" />
              <text fontSize={size} textAnchor="middle" style={style}>
                <textPath href={`#preview-line-${i}`} startOffset="50%">
                  {line.trim()}
                </textPath>
              </text>
            </g>
          );
        })}
      </svg>
    </div>
  );
}