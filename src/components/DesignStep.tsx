import { useEffect, useRef, useState } from "react";
import { Image as ImageIcon, PenLine, Trash2, Type } from "lucide-react";
import { isFixed } from "../lib/design";
import { ACCEPTED_FILE_TYPES } from "../lib/image";
import { inBrowser, inkscapeFound, isTauri } from "../lib/ipc";
import { artworkRect, everythingRect, itemRect, stockRect, useProject } from "../state/project";
import { useUi } from "../state/ui";
import { Checkbox, NumberField, Section } from "./controls";
import { Gallery } from "./Gallery";

const primary = "rounded-lg bg-accent px-3 py-2 font-medium text-accent-fg transition-opacity hover:opacity-90";
const choice =
  "rounded-lg border border-line px-3 py-2 text-left transition-colors hover:border-accent hover:text-accent disabled:opacity-40";
const quiet = "rounded-lg border border-line px-3 py-1.5 transition-colors hover:text-accent disabled:opacity-40";

/**
 * The design: what goes on the material. It starts with a picture, a drawing,
 * words or a box, and more words and drawings can then be added and arranged
 * around it.
 */
export function DesignStep() {
  const image = useProject((s) => s.image);
  const design = useProject((s) => s.design);
  const items = useProject((s) => s.items);
  const selected = useProject((s) => s.selected);
  const error = useProject((s) => s.error);
  const galleryError = useProject((s) => s.galleryError);
  const mode = useProject((s) => s.mode);
  const actions = useProject.getState();
  const input = useRef<HTMLInputElement>(null);
  // With Inkscape installed, more kinds of drawing can be opened.
  const [inkscape, setInkscape] = useState(false);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (isTauri()) inkscapeFound().then(setInkscape, () => setInkscape(false));
  }, []);

  /** Runs something that opens a file, showing why if it could not. */
  async function open(work: () => Promise<void>) {
    setBusy(true);
    try {
      await work();
    } catch (e) {
      useProject.setState({ error: e instanceof Error ? e.message : String(e) });
    } finally {
      setBusy(false);
    }
  }
  const drawings = inkscape ? "SVG, PDF, AI, EPS" : "SVG";
  const drawingTip = inkscape
    ? "Open an SVG, PDF, AI or EPS file and use its lines exactly as drawn. Inkscape converts the last three."
    : inBrowser
      ? "Open an SVG file and use its lines exactly as drawn. PDF, AI and EPS files need the desktop app, with Inkscape installed."
      : "Open an SVG file and use its lines exactly as drawn. With Inkscape installed, PDF, AI and EPS files can be opened too.";

  return (
    <div className="flex flex-col gap-5">
      <input
        ref={input}
        type="file"
        accept={ACCEPTED_FILE_TYPES}
        className="hidden"
        onChange={(e) => {
          const file = e.target.files?.[0];
          if (file) void actions.importFile(file);
          e.target.value = "";
        }}
      />
      {error && (
        <p role="alert" className="rounded-lg border border-accent/40 px-3 py-2 text-accent">
          {error}
        </p>
      )}

      {!image ? (
        <Section title="Start with">
          <button onClick={() => input.current?.click()} className={primary}>
            Choose image…
          </button>
          <p className="text-muted">
            Or drop a file anywhere on the canvas; an SVG is used as it is drawn, with no tracing. A
            picture copied in your web browser (right-click, Copy image) can be pasted with Ctrl+V, and
            so can a picture's web address.
          </p>
          <button onClick={() => void open(actions.openDrawing)} disabled={busy} title={drawingTip} className={choice}>
            {busy ? "Opening…" : `Open a drawing… (${drawings})`}
          </button>
          <button
            onClick={() => useUi.getState().setMaker("text")}
            title="Type words and draw them in any font on this computer"
            className={choice}
          >
            Text…
          </button>
          <button
            onClick={() => useUi.getState().setMaker("box")}
            title="Draw the panels of a box with finger joints, from its size and your material's thickness"
            className={choice}
          >
            Box maker…
          </button>
          <p className="text-muted">Once something is on the material, you can add words and drawings around it.</p>
        </Section>
      ) : (
        <>
          <Section title="On the material">
            <p className="text-muted">
              Click something here or on the canvas to select it, then drag it to move it or its handles
              to resize it.
            </p>
            <ul className="flex flex-col gap-1">
              <Row
                icon={design ? PenLine : ImageIcon}
                name={image.name.replace(/\.[^.]+$/, "")}
                active={selected === null}
                onSelect={() => actions.select(null)}
              />
              {items.map((item) => (
                <Row
                  key={item.id}
                  icon={item.text ? Type : PenLine}
                  name={item.name}
                  active={selected === item.id}
                  onSelect={() => actions.select(item.id)}
                  onRemove={() => actions.removeItem(item.id)}
                />
              ))}
            </ul>
            <div className="grid grid-cols-2 gap-2">
              <button
                onClick={() => useUi.getState().setMaker("text", "new")}
                title="Add words to the material, straight or along a curve"
                className={quiet}
              >
                Add text…
              </button>
              <button onClick={() => void open(actions.addDrawing)} disabled={busy} title={drawingTip} className={quiet}>
                Add a drawing…
              </button>
            </div>
          </Section>

          <Selected onReplace={() => input.current?.click()} />

          <button
            onClick={() => actions.setStep(mode === "simple" ? "operations" : "trace")}
            title={mode === "simple" ? "Go on to setting up the job" : "Go on to tracing the picture"}
            className={primary}
          >
            Continue
          </button>
        </>
      )}

      {galleryError && (
        <p role="alert" className="rounded-lg border border-accent/40 px-3 py-2 text-accent">
          {galleryError}
        </p>
      )}
      <Gallery />
    </div>
  );
}

interface RowProps {
  icon: typeof Type;
  name: string;
  active: boolean;
  onSelect: () => void;
  onRemove?: () => void;
}

function Row({ icon: Icon, name, active, onSelect, onRemove }: RowProps) {
  return (
    <li className={`flex items-center rounded-lg border ${active ? "border-accent bg-raised" : "border-line"}`}>
      <button onClick={onSelect} title={`Select ${name}`} className="flex min-w-0 flex-1 items-center gap-2 px-2.5 py-1.5 text-left">
        <Icon size={14} className={`shrink-0 ${active ? "text-accent" : "text-muted"}`} />
        <span className="truncate">{name}</span>
      </button>
      {onRemove && (
        <button onClick={onRemove} aria-label={`Remove ${name}`} className="rounded p-1.5 text-muted hover:text-accent">
          <Trash2 size={13} />
        </button>
      )}
    </li>
  );
}

/** The size and place of whatever is selected, and what can be done to it. */
function Selected({ onReplace }: { onReplace: () => void }) {
  const s = useProject();
  const item = s.items.find((i) => i.id === s.selected) ?? null;
  const fixed = !item && s.design !== null && isFixed(s.design);
  const edge = s.origin === "backLeft" ? "From back" : "From front";
  const stock = stockRect(s);
  const all = everythingRect(s);
  const overhangs =
    all !== null &&
    (all.x < stock.x - 0.01 ||
      all.y < stock.y - 0.01 ||
      all.x + all.width > stock.x + stock.width + 0.01 ||
      all.y + all.height > stock.y + stock.height + 0.01);

  const size = item ? { width: item.widthMm, height: item.heightMm } : { width: s.widthMm, height: s.heightMm };
  const place = item ? { left: item.left, front: item.front } : { left: s.left, front: s.front };
  const locked = item ? item.aspectLocked : s.aspectLocked;
  const setSize = (patch: { widthMm?: number; heightMm?: number }) => {
    if (item) s.updateItem(item.id, patch);
    else if (patch.widthMm !== undefined) s.setWidthMm(patch.widthMm);
    else if (patch.heightMm !== undefined) s.setHeightMm(patch.heightMm);
  };
  const setPlace = (patch: { left?: number; front?: number }) => (item ? s.updateItem(item.id, patch) : s.setPosition(patch));
  const centre = () =>
    setPlace({
      left: Math.round(((s.stockWidth - size.width) / 2) * 10) / 10,
      front: Math.round(((s.stockLength - size.height) / 2) * 10) / 10,
    });
  const name = item ? item.name : (s.image?.name.replace(/\.[^.]+$/, "") ?? "");
  const picture = !item && !s.design;

  // A box to fit the selected thing into, starting as the material less a margin.
  const [box, setBox] = useState(() => ({ width: Math.max(10, s.stockWidth - 20), height: Math.max(10, s.stockLength - 20) }));
  const [stretch, setStretch] = useState(false);
  /** Sizes the selected thing to a box: as large as fits inside it, or stretched to fill it exactly. */
  function fit(target: { width: number; height: number }, onMaterial: boolean) {
    const rect = item ? itemRect(item, s) : artworkRect(s);
    if (!rect) return;
    const scale = Math.min(target.width / rect.width, target.height / rect.height);
    const [width, height] = stretch ? [target.width, target.height] : [rect.width * scale, rect.height * scale];
    // It stays centred where it was, or goes to the middle of the material.
    const [cx, cy] = onMaterial ? [stock.x + stock.width / 2, stock.y + stock.height / 2] : [rect.x + rect.width / 2, rect.y + rect.height / 2];
    const next = { x: cx - width / 2, y: cy - height / 2, width, height };
    if (item) {
      if (stretch) s.updateItem(item.id, { aspectLocked: false });
      s.setItemRect(item.id, next);
    } else {
      if (stretch) s.setAspectLocked(false);
      s.setArtworkRect(next);
    }
  }

  return (
    <Section title={`Selected: ${name}`}>
      {fixed ? (
        <p className="text-muted">It stays the size it was drawn at, so that its parts fit. It can be moved.</p>
      ) : (
        <>
          <NumberField label="Width" unit="mm" min={1} max={5000} value={size.width} onChange={(widthMm) => setSize({ widthMm })} />
          <NumberField label="Height" unit="mm" min={1} max={5000} value={size.height} onChange={(heightMm) => setSize({ heightMm })} />
          <Checkbox
            label="Keep proportions"
            checked={locked}
            onChange={(on) => (item ? s.updateItem(item.id, { aspectLocked: on }) : s.setAspectLocked(on))}
          />
          <div className="flex flex-col gap-3 rounded-lg border border-line p-3">
            <span className="text-[11px] font-semibold tracking-wider text-muted uppercase">Fit into a box</span>
            <NumberField
              label="Box width"
              unit="mm"
              step={1}
              min={1}
              max={5000}
              value={box.width}
              hint={{ text: "The width of the space to fit the selected thing into." }}
              onChange={(width) => setBox((b) => ({ ...b, width }))}
            />
            <NumberField
              label="Box height"
              unit="mm"
              step={1}
              min={1}
              max={5000}
              value={box.height}
              hint={{ text: "The height of the space to fit the selected thing into." }}
              onChange={(height) => setBox((b) => ({ ...b, height }))}
            />
            <Checkbox
              label="Stretch to fill it exactly"
              hint={{ text: "Off, it is made as large as fits inside the box, keeping its shape. On, it is stretched or squeezed to the box's exact width and height, which changes the shape of letters and pictures." }}
              checked={stretch}
              onChange={setStretch}
            />
            <div className="flex flex-wrap gap-2">
              <button className={quiet} onClick={() => fit(box, false)} title="Size it to this box, keeping it centred where it is">
                Fit to the box
              </button>
              <button
                className={quiet}
                onClick={() => fit({ width: Math.max(1, s.stockWidth - 20), height: Math.max(1, s.stockLength - 20) }, true)}
                title="Size it to the material, less a 10 mm margin, and centre it there"
              >
                Fill the material
              </button>
            </div>
          </div>
        </>
      )}
      <NumberField label="From left" unit="mm" min={-5000} max={5000} value={place.left} onChange={(left) => setPlace({ left })} />
      <NumberField label={edge} unit="mm" min={-5000} max={5000} value={place.front} onChange={(front) => setPlace({ front })} />
      {overhangs && (
        <p role="alert" className="text-accent">
          Something reaches past the edge of the material.
        </p>
      )}
      <div className="flex flex-wrap gap-2">
        <button className={quiet} onClick={centre} title="Put it in the middle of the material">
          Centre it
        </button>
        {item?.text && (
          <button className={quiet} onClick={() => useUi.getState().setMaker("text", item.id)}>
            Change the text…
          </button>
        )}
        {!item && s.design && s.design.kind !== "drawing" && (
          <button className={quiet} onClick={() => useUi.getState().setMaker(s.design!.kind as "box" | "layers" | "text")}>
            {s.design.kind === "box" ? "Change the box…" : s.design.kind === "layers" ? "Change the layers…" : "Change the text…"}
          </button>
        )}
        {picture && (
          <>
            <button className={quiet} onClick={() => s.setCropping(!s.cropping)}>
              {s.cropping ? "Cancel crop" : "Crop…"}
            </button>
            <button className={quiet} onClick={() => useUi.getState().setMaker("background")}>
              Remove background…
            </button>
            <button className={quiet} disabled={s.uncropped === null} onClick={() => void s.resetCrop()}>
              Restore the picture
            </button>
            <button
              className={quiet}
              onClick={() => useUi.getState().setMaker("layers")}
              title="Slice the picture into layers to cut from thin sheet and stack"
            >
              Stacked layers…
            </button>
          </>
        )}
        {!item && (
          <>
            <button className={quiet} onClick={onReplace} title="Pick a different picture or SVG in its place; added text and drawings stay">
              Replace…
            </button>
            <button className={quiet} onClick={s.clearImage} title="Clear the material and start the design again">
              Remove everything
            </button>
          </>
        )}
        {item && (
          <button className={quiet} onClick={() => s.removeItem(item.id)} title="Take this off the material">
            Remove
          </button>
        )}
      </div>
      {artworkRect(s) === null && <p className="text-muted">Nothing is on the material yet.</p>}
    </Section>
  );
}
