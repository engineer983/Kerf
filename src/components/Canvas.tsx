import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Upload } from "lucide-react";
import { isFixed } from "../lib/design";
import { findMaterial, paintMaterial } from "../lib/materials";
import { reduceInto } from "../lib/reduce";
import { Simulation, type Rect } from "../lib/simulation";
import { setResultCanvas } from "../lib/thumbnail";
import { PLAYBACK_SECONDS, usePlayback } from "../state/playback";
import { artworkRect, everythingRect, itemRect, jobOrder, jobUses, stockRect, useProject } from "../state/project";
import { useToolbox } from "../state/tools";
import { Handles } from "./Handles";
import { PlaybackBar } from "./PlaybackBar";
import { View3D } from "./View3D";

/** Top-left corner of the visible area in mm, and the zoom as mm per screen pixel. */
interface View {
  x: number;
  y: number;
  mmPerPx: number;
}

const MARGIN_MM = 30;
const MIN_MM_PER_PX = 0.01;
const MAX_MM_PER_PX = 5;
/** Bit assumed for router jobs whose operation has none chosen. */
const DEFAULT_BIT = { diameter: 3.175, kind: "flat" as const, angle: null, taperAngle: null, shank: 3.175 };

/** Something on the material that is not selected: a faint frame that selects it when clicked. */
function Target({ rect, px, onSelect }: { rect: Rect; px: number; onSelect: () => void }) {
  return (
    <rect
      x={rect.x}
      y={rect.y}
      width={rect.width}
      height={rect.height}
      fill="transparent"
      stroke="var(--muted)"
      strokeWidth={px}
      strokeDasharray={`${px * 3} ${px * 3}`}
      style={{ pointerEvents: "auto", cursor: "pointer" }}
      onPointerDown={(e) => {
        if (e.button !== 0) return;
        // Selecting must not also pan the canvas underneath.
        e.stopPropagation();
        onSelect();
      }}
      onDoubleClick={(e) => e.stopPropagation()}
    />
  );
}

export function Canvas() {
  const bed = useProject((s) => s.bed);
  const image = useProject((s) => s.image);
  const importFile = useProject((s) => s.importFile);
  const widthMm = useProject((s) => s.widthMm);
  const heightMm = useProject((s) => s.heightMm);
  const aspectLocked = useProject((s) => s.aspectLocked);
  const cropping = useProject((s) => s.cropping);
  const showSource = useProject((s) => s.showSource);
  const traced = useProject((s) => s.trace.result);
  const traceMode = useProject((s) => s.trace.settings.mode);
  const adjust = useProject((s) => s.trace.settings.preprocess);
  // A box or a set of layers is drawn as the lines to cut, with no picture behind.
  const designed = useProject((s) => s.design !== null);
  const fixedSize = useProject((s) => s.design !== null && isFixed(s.design));
  // Text and drawings added beside the main artwork, and which is selected.
  const items = useProject((s) => s.items);
  const selected = useProject((s) => s.selected);
  const selectedItem = items.find((item) => item.id === selected) ?? null;
  const left = useProject((s) => s.left);
  const front = useProject((s) => s.front);
  const program = useProject((s) => s.gcode.program);
  const materialId = useProject((s) => s.materialId);
  const thickness = useProject((s) => s.thickness);
  const machineKind = useProject((s) => s.machineKind);
  const operations = useProject((s) => s.operations);
  const origin = useProject((s) => s.origin);
  const stockWidth = useProject((s) => s.stockWidth);
  const stockLength = useProject((s) => s.stockLength);
  const tools = useToolbox((s) => s.tools);
  const mode = usePlayback((s) => s.mode);
  const playing = usePlayback((s) => s.playing);
  const progress = usePlayback((s) => s.progress);
  const compare = usePlayback((s) => s.compare);

  // A job that engraves or carves the picture itself is shown as the picture,
  // in the greys it will be read as. The traced shapes are drawn only when
  // something follows them.
  const uses = jobUses(operations);
  const showTraced = designed || uses.traced || !uses.picture;
  const gain = (1 + Math.min(0.999, Math.max(-1, adjust.contrast))) / (1 - Math.min(0.999, Math.max(-1, adjust.contrast)));
  const lift = 0.5 - 0.5 * gain + adjust.brightness;

  const containerRef = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ w: 0, h: 0 });
  const [view, setView] = useState<View | null>(null);
  const [dragOver, setDragOver] = useState(false);
  const pan = useRef<{ x: number; y: number } | null>(null);
  /** Whether the person has zoomed or moved the view since it was last fitted. */
  const moved = useRef(false);

  useLayoutEffect(() => {
    const el = containerRef.current!;
    const observer = new ResizeObserver(([entry]) => {
      setSize({ w: entry.contentRect.width, h: entry.contentRect.height });
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  const fit = useCallback(() => {
    if (size.w === 0 || size.h === 0) return;
    moved.current = false;
    const mmPerPx = Math.max(
      (bed.width + MARGIN_MM * 2) / size.w,
      (bed.height + MARGIN_MM * 2) / size.h,
    );
    setView({
      x: bed.width / 2 - (size.w * mmPerPx) / 2,
      y: bed.height / 2 - (size.h * mmPerPx) / 2,
      mmPerPx,
    });
  }, [bed, size]);

  // A different bed size invalidates the view; clearing it triggers a new fit.
  useEffect(() => setView(null), [bed.width, bed.height]);

  // Fit when the container first has a size, and after the view is cleared.
  useEffect(() => {
    if (!view) fit();
  }, [view, fit]);

  // When the canvas changes size (a side panel is widened, say) the bed is
  // fitted to it again, unless the person has chosen their own view.
  useEffect(() => {
    if (!moved.current) fit();
    // Only a change of size should do this, not every new `fit`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [size.w, size.h]);

  function onWheel(e: React.WheelEvent) {
    if (!view) return;
    const rect = containerRef.current!.getBoundingClientRect();
    const px = e.clientX - rect.left;
    const py = e.clientY - rect.top;
    const mmPerPx = Math.min(
      MAX_MM_PER_PX,
      Math.max(MIN_MM_PER_PX, view.mmPerPx * Math.exp(e.deltaY * 0.0015)),
    );
    moved.current = true;
    // Keep the point under the cursor fixed while zooming.
    setView({
      x: view.x + px * (view.mmPerPx - mmPerPx),
      y: view.y + py * (view.mmPerPx - mmPerPx),
      mmPerPx,
    });
  }

  function onPointerDown(e: React.PointerEvent) {
    if (e.button !== 0 && e.button !== 1) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    pan.current = { x: e.clientX, y: e.clientY };
  }

  function onPointerMove(e: React.PointerEvent) {
    if (!pan.current || !view) return;
    const dx = e.clientX - pan.current.x;
    const dy = e.clientY - pan.current.y;
    pan.current = { x: e.clientX, y: e.clientY };
    if (dx !== 0 || dy !== 0) moved.current = true;
    setView({ ...view, x: view.x - dx * view.mmPerPx, y: view.y - dy * view.mmPerPx });
  }

  function onPointerUp() {
    pan.current = null;
  }

  function onDrop(e: React.DragEvent) {
    e.preventDefault();
    setDragOver(false);
    const file = e.dataTransfer.files[0];
    if (file) void importFile(file);
  }

  const placed = artworkRect({ image, widthMm, heightMm, left, front, bed, origin });
  // What a cutout goes round: the artwork and everything added to it.
  const around = everythingRect({ image, widthMm, heightMm, left, front, bed, origin, items });

  // The crop frame, in bed coordinates, while a crop is being chosen.
  const [cropRect, setCropRect] = useState<Rect | null>(null);
  useEffect(() => {
    const rect = artworkRect(useProject.getState());
    // Start a little inside the artwork, so the frame is easy to see and grab.
    setCropRect(
      cropping && rect
        ? {
            x: rect.x + rect.width * 0.1,
            y: rect.y + rect.height * 0.1,
            width: rect.width * 0.8,
            height: rect.height * 0.8,
          }
        : null,
    );
  }, [cropping]);

  const toBed = useCallback(
    (e: { clientX: number; clientY: number }): [number, number] => {
      const box = containerRef.current!.getBoundingClientRect();
      const v = view ?? { x: 0, y: 0, mmPerPx: 1 };
      return [v.x + (e.clientX - box.left) * v.mmPerPx, v.y + (e.clientY - box.top) * v.mmPerPx];
    },
    [view],
  );

  function applyCrop() {
    if (!placed || !cropRect) return;
    void useProject.getState().applyCrop({
      x0: (cropRect.x - placed.x) / placed.width,
      y0: (cropRect.y - placed.y) / placed.height,
      x1: (cropRect.x + cropRect.width - placed.x) / placed.width,
      y1: (cropRect.y + cropRect.height - placed.y) / placed.height,
    });
  }
  const px = view?.mmPerPx ?? 1;

  // Ruler numbers are spaced so they stay readable at any zoom: every 10 mm
  // when zoomed in, further apart when zoomed out.
  const rulerStep = [10, 20, 50, 100, 200, 500].find((step) => step / px >= 44) ?? 1000;
  const ticks = (length: number) =>
    Array.from({ length: Math.floor(length / rulerStep) + 1 }, (_, i) => i * rulerStep);

  const material = findMaterial(materialId);
  const stock = stockRect({ stockWidth, stockLength, bed, origin });
  const [sx, sy, sw, sh] = image ? [stock.x, stock.y, stock.width, stock.height] : [0, 0, 0, 0];

  // The bare material, as an image for the 2D view.
  const materialUrl = useMemo(() => {
    if (sw <= 0 || sh <= 0) return null;
    const pxPerMm = Math.min(4, 1024 / Math.max(sw, sh));
    const canvas = document.createElement("canvas");
    canvas.width = Math.max(2, Math.round(sw * pxPerMm));
    canvas.height = Math.max(2, Math.round(sh * pxPerMm));
    paintMaterial(canvas, material, pxPerMm);
    return canvas.toDataURL();
  }, [material, sw, sh]);

  // The preview carves with one bit: the first router operation's.
  const routing = operations.find((o) => o.kind.type === "cncRelief" || o.kind.type === "cncProfile");
  // A carving is made without sharpening, so the picture is shown without it too.
  const sharpen = operations.some((o) => o.kind.type === "cncRelief" && o.enabled !== false) ? 0 : Math.min(2, adjust.sharpen ?? 0);
  const chosen = tools.find((t) => t.id === routing?.toolId);
  const bit =
    chosen ??
    (routing?.kind.type === "cncRelief"
      ? {
          diameter: routing.kind.toolDiameter,
          kind: routing.kind.ballNose ? ("ball" as const) : ("flat" as const),
          angle: null,
          taperAngle: null,
          shank: routing.kind.toolDiameter,
        }
      : DEFAULT_BIT);

  const engraving = operations.find((o) => o.kind.type === "laserImage")?.kind;
  const imageInterval = engraving?.type === "laserImage" ? engraving.interval : undefined;
  const simulation = useMemo(() => {
    if (!program || sw <= 0 || sh <= 0) return null;
    return new Simulation({
      moves: program.moves,
      stock: { x: sx, y: sy, width: sw, height: sh },
      thickness,
      material,
      kind: machineKind,
      tool: {
        diameter: bit.diameter,
        kind: bit.kind,
        angle: bit.angle,
        taperAngle: bit.taperAngle,
        shank: bit.shank,
      },
      imageInterval,
      // Each included router operation carves with its own bit.
      tools: jobOrder(operations, items.length > 0).all.map((o) => tools.find((t) => t.id === o.toolId) ?? bit),
      operationStarts: program.operationStarts,
    });
    // The operations' settings are read as they were when the program was made:
    // any change to them clears the program, so they cannot be out of step.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [program, sx, sy, sw, sh, thickness, material, machineKind]);

  // Tool position for the 2D marker, refreshed whenever the playback point moves.
  const [head, setHead] = useState<{ x: number; y: number; cutting: boolean } | null>(null);
  useLayoutEffect(() => {
    if (!simulation) {
      setHead(null);
      return;
    }
    simulation.seek(progress);
    setHead({ x: simulation.head[0], y: simulation.head[1], cutting: simulation.cutting });
  }, [simulation, progress]);

  useEffect(() => {
    if (!playing || !simulation) return;
    let last = performance.now();
    let frame = requestAnimationFrame(function step(now) {
      const { progress, speed, setProgress, finish } = usePlayback.getState();
      const next = progress + ((now - last) / 1000 / PLAYBACK_SECONDS) * speed;
      last = now;
      if (next >= 1) {
        finish();
        return;
      }
      setProgress(next);
      frame = requestAnimationFrame(step);
    });
    return () => cancelAnimationFrame(frame);
  }, [playing, simulation]);

  // In the result view the simulation's own canvas is shown over the material.
  const resultHost = useRef<HTMLDivElement>(null);
  const showResult = simulation !== null && mode === "result";
  // It is far finer than the screen, so a copy reduced to the size it is
  // seen at is what is shown: see `reduceInto`.
  const shown = useMemo(() => document.createElement("canvas"), []);
  /** How many of the screen's own pixels the material is wide at this zoom. */
  const shownWidth = (sw / px) * window.devicePixelRatio;
  useLayoutEffect(() => {
    if (!showResult || !resultHost.current) return;
    shown.style.cssText = "display:block;width:100%;height:100%";
    resultHost.current.replaceChildren(shown);
  }, [showResult, shown]);
  // After the layout effect above that moves the simulation to `progress`.
  useLayoutEffect(() => {
    if (showResult) reduceInto(shown, simulation.canvas, shownWidth);
  }, [showResult, simulation, progress, shownWidth, shown]);

  // The gallery pictures a project by its simulated result.
  useEffect(() => {
    setResultCanvas(simulation?.canvas ?? null);
    return () => setResultCanvas(null);
  }, [simulation]);

  // How the previous result ended up, kept so that a change can be compared
  // against it side by side.
  const [before, setBefore] = useState<{ canvas: HTMLCanvasElement; width: number; height: number } | null>(null);
  useEffect(() => {
    if (!simulation) return;
    // Runs when this simulation is replaced or dropped.
    return () => {
      simulation.seek(1);
      const copy = document.createElement("canvas");
      copy.width = simulation.canvas.width;
      copy.height = simulation.canvas.height;
      copy.getContext("2d")!.drawImage(simulation.canvas, 0, 0);
      setBefore({ canvas: copy, width: simulation.setup.stock.width, height: simulation.setup.stock.height });
    };
  }, [simulation]);

  // The comparison only makes sense on the same piece of material.
  const comparable = before !== null && before.width === sw && before.height === sh;
  const comparing = showResult && compare && comparable;
  /** Where the divider sits across the material, 0 at the left and 1 at the right. */
  const [split, setSplit] = useState(0.5);
  const beforeHost = useRef<HTMLDivElement>(null);
  const shownBefore = useMemo(() => document.createElement("canvas"), []);
  useLayoutEffect(() => {
    if (!comparing || !beforeHost.current || !before) return;
    shownBefore.style.cssText = "display:block;width:100%;height:100%";
    beforeHost.current.replaceChildren(shownBefore);
    reduceInto(shownBefore, before.canvas, shownWidth);
  }, [comparing, before, shownWidth, shownBefore]);

  function dragSplit(e: React.PointerEvent) {
    if (!e.currentTarget.hasPointerCapture(e.pointerId) || !view) return;
    const [x] = toBed(e);
    setSplit(Math.min(1, Math.max(0, (x - sx) / sw)));
  }
  return (
    <main
      ref={containerRef}
      className="relative min-w-0 flex-1 cursor-grab overflow-hidden bg-bg active:cursor-grabbing"
      onWheel={onWheel}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
      onDoubleClick={fit}
      onDragOver={(e) => {
        e.preventDefault();
        setDragOver(true);
      }}
      onDragLeave={() => setDragOver(false)}
      onDrop={onDrop}
    >
      {view && (
        <svg
          className="absolute inset-0 h-full w-full"
          viewBox={`${view.x} ${view.y} ${size.w * view.mmPerPx} ${size.h * view.mmPerPx}`}
        >
          <defs>
            <pattern id="grid-minor" width="10" height="10" patternUnits="userSpaceOnUse">
              <path d="M10 0H0V10" fill="none" stroke="var(--grid)" strokeWidth={px} />
            </pattern>
            <pattern id="grid-major" width="50" height="50" patternUnits="userSpaceOnUse">
              <rect width="50" height="50" fill="url(#grid-minor)" />
              <path d="M50 0H0V50" fill="none" stroke="var(--grid-major)" strokeWidth={px} />
            </pattern>
          </defs>
          <rect width={bed.width} height={bed.height} fill="var(--bed)" />
          <rect width={bed.width} height={bed.height} fill="url(#grid-major)" />
          <rect
            width={bed.width}
            height={bed.height}
            fill="none"
            stroke="var(--line)"
            strokeWidth={px * 1.5}
          />
          {/* Numbered lines, measured from the machine's X0 Y0 corner. */}
          {ticks(bed.width).map((v) => (
            <g key={`x${v}`}>
              <line x1={v} x2={v} y1={0} y2={bed.height} stroke="var(--grid-major)" strokeWidth={px * 1.25} />
              <text
                x={v}
                y={origin === "backLeft" ? -6 * px : bed.height + 14 * px}
                fontSize={10 * px}
                textAnchor="middle"
                fill="var(--muted)"
                className="font-mono"
              >
                {v}
              </text>
            </g>
          ))}
          {ticks(bed.height).map((v) => {
            const y = origin === "backLeft" ? v : bed.height - v;
            return (
              <g key={`y${v}`}>
                <line x1={0} x2={bed.width} y1={y} y2={y} stroke="var(--grid-major)" strokeWidth={px * 1.25} />
                <text
                  x={-6 * px}
                  y={y + 3.5 * px}
                  fontSize={10 * px}
                  textAnchor="end"
                  fill="var(--muted)"
                  className="font-mono"
                >
                  {v}
                </text>
              </g>
            );
          })}
          {materialUrl && (
            <image href={materialUrl} x={sx} y={sy} width={sw} height={sh} preserveAspectRatio="none" />
          )}
          {uses.picture && image && placed && (
            // The same steps the backend takes: to grey, then contrast and
            // brightness, then blur, then sharpening.
            <filter id="kerf-as-read" colorInterpolationFilters="sRGB" x="0" y="0" width="1" height="1">
              <feColorMatrix
                type="matrix"
                values="0.299 0.587 0.114 0 0  0.299 0.587 0.114 0 0  0.299 0.587 0.114 0 0  0 0 0 1 0"
              />
              <feComponentTransfer result="toned">
                <feFuncR type="linear" slope={gain} intercept={lift} />
                <feFuncG type="linear" slope={gain} intercept={lift} />
                <feFuncB type="linear" slope={gain} intercept={lift} />
              </feComponentTransfer>
              {adjust.blurSigma > 0 && <feGaussianBlur stdDeviation={(adjust.blurSigma * placed.width) / image.width} result="read" />}
              {sharpen > 0 && (
                <>
                  {/* An unsharp mask: each point is pushed away from the average of what surrounds it. */}
                  <feGaussianBlur
                    stdDeviation={Math.min(8, Math.max(0.8, Math.max(image.width, image.height) / 560)) * (placed.width / image.width)}
                    edgeMode="duplicate"
                    result="around"
                  />
                  <feComposite
                    in={adjust.blurSigma > 0 ? "read" : "toned"}
                    in2="around"
                    operator="arithmetic"
                    k2={1 + sharpen}
                    k3={-sharpen}
                  />
                </>
              )}
            </filter>
          )}
          {image && placed && !designed && (uses.picture || showSource || !traced || !showTraced) && (
            <image
              href={image.url}
              x={placed.x}
              y={placed.y}
              width={placed.width}
              height={placed.height}
              preserveAspectRatio="none"
              filter={uses.picture ? "url(#kerf-as-read)" : undefined}
              // Faded under traced shapes, and under a generated job's paths.
              opacity={uses.picture ? (program ? 0.5 : 1) : traced ? 0.25 : 1}
            />
          )}
          {traced && placed && showTraced && (
            // The markup is generated by our own tracer from numeric path data.
            <svg
              x={placed.x}
              y={placed.y}
              width={placed.width}
              height={placed.height}
              viewBox={`0 0 ${traced.width} ${traced.height}`}
              preserveAspectRatio="none"
              className={designed ? "design-lines" : traceMode === "outline" ? "trace-mono" : undefined}
              opacity={program ? 0.15 : 1}
              dangerouslySetInnerHTML={{ __html: traced.body }}
            />
          )}
          {items.map((item) => {
            const at = itemRect(item, { bed, origin });
            return (
              // Lines made by our own text and drawing readers, from numeric path data.
              <svg
                key={item.id}
                x={at.x}
                y={at.y}
                width={at.width}
                height={at.height}
                viewBox={`0 0 ${item.drawnWidth} ${item.drawnHeight}`}
                preserveAspectRatio="none"
                className="design-lines"
                opacity={program ? 0.15 : 1}
                dangerouslySetInnerHTML={{ __html: item.body }}
              />
            );
          })}
          {/* The line each cutout follows, round everything on the material, before any allowance for the bit's width. */}
          {around &&
            operations.map((o) => {
              if ((o.kind.type !== "cncCutout" && o.kind.type !== "laserCutout") || o.enabled === false) return null;
              const { offset: margin, outline, cornerRadius } = o.kind;
              const [w, h] = [around.width + margin * 2, around.height + margin * 2];
              if (w <= 0 || h <= 0) return null;
              const line = { fill: "none", stroke: "var(--fg)", strokeWidth: px * 1.5, strokeDasharray: `${px * 6} ${px * 4}` };
              return outline === "ellipse" ? (
                <ellipse key={o.id} cx={around.x + around.width / 2} cy={around.y + around.height / 2} rx={w / 2} ry={h / 2} {...line} />
              ) : (
                <rect key={o.id} x={around.x - margin} y={around.y - margin} width={w} height={h} rx={Math.min(cornerRadius, w / 2, h / 2)} {...line} />
              );
            })}
          {program && (
            <>
              <path
                d={program.travelPath}
                fill="none"
                stroke="var(--muted)"
                strokeWidth={px}
                strokeDasharray={`${px * 4} ${px * 4}`}
                opacity={0.6}
              />
              <path d={program.cutPath} fill="none" stroke="var(--accent)" strokeWidth={px * 1.25} />
            </>
          )}
        </svg>
      )}

      {view && showResult && (
        <div
          ref={resultHost}
          className="pointer-events-none absolute"
          style={{
            left: (sx - view.x) / px,
            top: (sy - view.y) / px,
            width: sw / px,
            height: sh / px,
          }}
        />
      )}
      {view && comparing && (
        <>
          {/* The earlier result, shown only to the left of the divider. */}
          <div
            ref={beforeHost}
            className="pointer-events-none absolute"
            style={{
              left: (sx - view.x) / px,
              top: (sy - view.y) / px,
              width: sw / px,
              height: sh / px,
              clipPath: `inset(0 ${(1 - split) * 100}% 0 0)`,
            }}
          />
          <div
            className="absolute flex w-4 -translate-x-1/2 cursor-ew-resize justify-center"
            title="Drag to compare the result before and after your changes"
            style={{ left: (sx + sw * split - view.x) / px, top: (sy - view.y) / px, height: sh / px }}
            onPointerDown={(e) => {
              e.stopPropagation();
              e.currentTarget.setPointerCapture(e.pointerId);
            }}
            onPointerMove={dragSplit}
            onDoubleClick={(e) => e.stopPropagation()}
          >
            <div className="h-full w-0.5 bg-accent shadow" />
            <div className="absolute top-1/2 h-8 w-3 -translate-y-1/2 rounded-full border border-white bg-accent" />
          </div>
          <div
            className="pointer-events-none absolute flex -translate-y-full justify-between pb-1 text-[11px] font-medium text-muted"
            style={{ left: (sx - view.x) / px, top: (sy - view.y) / px, width: sw / px }}
          >
            <span>Before</span>
            <span>After</span>
          </div>
        </>
      )}
      {view && showResult && head && (
        <div
          className={`pointer-events-none absolute h-3 w-3 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 ${
            head.cutting ? "border-white bg-red-500" : "border-white/80 bg-transparent"
          }`}
          style={{ left: (head.x - view.x) / px, top: (head.y - view.y) / px }}
        />
      )}
      {view && placed && !(simulation && mode === "3d") && (
        // A second layer, above the result picture, so the frame stays visible
        // and can be grabbed in every 2D view.
        <svg
          className="pointer-events-none absolute inset-0 h-full w-full"
          viewBox={`${view.x} ${view.y} ${size.w * view.mmPerPx} ${size.h * view.mmPerPx}`}
        >
          {cropRect ? (
            <>
              <path
                fillRule="evenodd"
                fill="rgba(0, 0, 0, 0.55)"
                d={`M${placed.x} ${placed.y}h${placed.width}v${placed.height}h${-placed.width}zM${cropRect.x} ${cropRect.y}h${cropRect.width}v${cropRect.height}h${-cropRect.width}z`}
              />
              <Handles rect={cropRect} px={px} lockAspect={false} bounds={placed} dashed toBed={toBed} onChange={setCropRect} />
            </>
          ) : (
            <>
              {/* Whatever is selected has the frame; anything else is clicked to select it. */}
              {selectedItem && <Target rect={placed} px={px} onSelect={() => useProject.getState().select(null)} />}
              {selectedItem ? (
                <Handles
                  rect={itemRect(selectedItem, { bed, origin })}
                  px={px}
                  lockAspect={selectedItem.aspectLocked}
                  toBed={toBed}
                  onChange={(rect) => useProject.getState().setItemRect(selectedItem.id, rect)}
                />
              ) : (
                <Handles
                  rect={placed}
                  px={px}
                  lockAspect={aspectLocked}
                  moveOnly={fixedSize}
                  toBed={toBed}
                  onChange={useProject.getState().setArtworkRect}
                />
              )}
              {items
                .filter((item) => item.id !== selected)
                .map((item) => (
                  <Target
                    key={item.id}
                    rect={itemRect(item, { bed, origin })}
                    px={px}
                    onSelect={() => useProject.getState().select(item.id)}
                  />
                ))}
            </>
          )}
        </svg>
      )}
      {cropRect && (
        <div
          className="absolute top-3 left-1/2 flex -translate-x-1/2 cursor-default items-center gap-2 rounded-xl border border-line bg-surface/95 px-3 py-2 shadow-sm"
          onPointerDown={(e) => e.stopPropagation()}
          onDoubleClick={(e) => e.stopPropagation()}
        >
          <span className="text-muted">Drag the frame to choose what to keep</span>
          <button className="rounded-lg bg-accent px-3 py-1 font-medium text-accent-fg" onClick={applyCrop}>
            Apply crop
          </button>
          <button
            className="rounded-lg border border-line px-3 py-1 text-muted hover:text-fg"
            onClick={() => useProject.getState().setCropping(false)}
          >
            Cancel
          </button>
        </div>
      )}
      {simulation && mode === "3d" && <View3D simulation={simulation} bed={bed} />}
      {simulation && <PlaybackBar canCompare={comparable} />}

      {!image && (
        <div className="pointer-events-none absolute inset-0 flex items-center justify-center">
          <div className="flex flex-col items-center gap-3 rounded-2xl border border-line bg-surface/90 px-10 py-8 text-center shadow-sm backdrop-blur">
            <Upload size={22} strokeWidth={1.5} className="text-accent" />
            <div className="text-[15px] font-medium">Drop an image to begin</div>
            <div className="text-muted">PNG, JPEG, WebP, BMP or GIF</div>
          </div>
        </div>
      )}

      {dragOver && (
        <div className="pointer-events-none absolute inset-3 rounded-xl border-2 border-dashed border-accent" />
      )}

      <div className="pointer-events-none absolute top-3 left-3 rounded-md border border-line bg-surface/90 px-2 py-1 font-mono text-[11px] text-muted">
        {bed.width} × {bed.height} mm
        {view && ` · ${(100 / (view.mmPerPx * 3.7795)).toFixed(0)}%`}
      </div>
    </main>
  );
}

