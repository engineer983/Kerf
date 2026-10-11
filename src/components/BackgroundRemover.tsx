import { useEffect, useLayoutEffect, useReducer, useRef, useState, type PointerEvent } from "react";
import { Maximize, X, ZoomIn, ZoomOut } from "lucide-react";
import { cutOut, findSubject, maskCanvas, type Backdrop } from "../lib/subject";
import { useProject } from "../state/project";
import { useUi } from "../state/ui";
import { Segmented, Select, Slider } from "./controls";

const primary = "rounded-lg bg-accent px-4 py-2 font-medium text-accent-fg hover:opacity-90 disabled:opacity-40";
const quiet = "rounded-lg border border-line px-3 py-1.5 transition-colors hover:text-fg disabled:opacity-40";
const caption = "text-[11px] font-semibold tracking-wider text-muted uppercase";

/** Longest side, in pixels, of the copy that is drawn on. The finished picture keeps its full size. */
const WORK_MAX_PX = 2400;
const UNDO_STEPS = 16;
/** How far the picture can be magnified, as a multiple of the size that fits the window. */
const ZOOM_MAX = 16;
const ZOOM_STEP = 1.3;
/** Space left round the picture, in pixels, when it is fitted to the window. */
const MARGIN = 16;
/** Laid over what will be removed, so that it can still be seen and brought back. */
const WASH = "rgba(40, 110, 255, 0.62)";

type Point = [number, number];

interface Work {
  picture: HTMLImageElement;
  /** Opaque where the picture is kept, clear where it is removed. */
  mask: HTMLCanvasElement;
  scratch: HTMLCanvasElement;
}

const reason = (e: unknown) => (e instanceof Error ? e.message : String(e));

/**
 * Removes a picture's background. The subject is found automatically, and the
 * person corrects it by drawing a loop round, or brushing over, what should be
 * kept or removed.
 */
export function BackgroundRemover() {
  const image = useProject((s) => s.image);
  const display = useRef<HTMLCanvasElement>(null);
  const work = useRef<Work | null>(null);
  /** Earlier states of the mask, each as how much of every pixel is kept. */
  const history = useRef<Uint8ClampedArray[]>([]);
  const viewport = useRef<HTMLDivElement>(null);
  /** The size, in screen pixels, at which the whole picture fits the window. */
  const [fit, setFit] = useState<{ width: number; height: number } | null>(null);
  const [zoom, setZoom] = useState(1);
  /** The point of the picture (as fractions) to hold at a place in the window when the zoom changes. */
  const anchor = useRef<{ fx: number; fy: number; x: number; y: number } | null>(null);
  /** A drag that moves the view: where it started, and where the view was. */
  const pan = useRef<{ x: number; y: number; left: number; top: number } | null>(null);
  const spaceHeld = useRef(false);
  /** The loop or brush stroke being drawn. */
  const stroke = useRef<Point[] | null>(null);
  const pointer = useRef<Point | null>(null);
  const [, redraw] = useReducer((n: number) => n + 1, 0);
  const [status, setStatus] = useState<string | null>("Starting…");
  const [problem, setProblem] = useState<string | null>(null);
  const [mode, setMode] = useState<"keep" | "remove">("keep");
  const [tool, setTool] = useState<"loop" | "brush">("loop");
  const [brush, setBrush] = useState(3);
  const [view, setView] = useState<"edit" | "result">("edit");
  const [backdrop, setBackdrop] = useState<Backdrop>("white");
  const [canUndo, setCanUndo] = useState(false);
  const [working, setWorking] = useState(false);
  const close = () => useUi.getState().setMaker(null);

  const maskContext = () => work.current!.mask.getContext("2d", { willReadFrequently: true })!;

  /** Keeps the mask as it is now, to go back to. */
  function remember() {
    const { mask } = work.current!;
    const pixels = maskContext().getImageData(0, 0, mask.width, mask.height).data;
    const kept = new Uint8ClampedArray(mask.width * mask.height);
    for (let i = 0; i < kept.length; i++) kept[i] = pixels[i * 4 + 3];
    history.current.push(kept);
    if (history.current.length > UNDO_STEPS) history.current.shift();
    setCanUndo(true);
  }

  function undo() {
    const last = history.current.pop();
    if (!last || !work.current) return;
    const { mask } = work.current;
    const pixels = new ImageData(mask.width, mask.height);
    for (let i = 0; i < last.length; i++) {
      pixels.data[i * 4] = pixels.data[i * 4 + 1] = pixels.data[i * 4 + 2] = 255;
      pixels.data[i * 4 + 3] = last[i];
    }
    maskContext().putImageData(pixels, 0, 0);
    setCanUndo(history.current.length > 0);
    redraw();
  }

  /** Changes the zoom, holding the picture still at a point of the window (its middle unless given). */
  function zoomBy(factor: number, at?: { clientX: number; clientY: number }) {
    const view = viewport.current;
    const canvas = display.current;
    if (!view || !canvas) return;
    const frame = view.getBoundingClientRect();
    const box = canvas.getBoundingClientRect();
    const x = at ? at.clientX - frame.left : frame.width / 2;
    const y = at ? at.clientY - frame.top : frame.height / 2;
    anchor.current = {
      fx: (frame.left + x - box.left) / box.width,
      fy: (frame.top + y - box.top) / box.height,
      x,
      y,
    };
    setZoom((z) => Math.min(ZOOM_MAX, Math.max(1, z * factor)));
  }

  /** Keeps or removes the whole picture. */
  function setAll(kept: boolean) {
    const { mask } = work.current!;
    const context = maskContext();
    context.globalCompositeOperation = "source-over";
    context.clearRect(0, 0, mask.width, mask.height);
    if (kept) {
      context.fillStyle = "#fff";
      context.fillRect(0, 0, mask.width, mask.height);
    }
  }

  async function find(url: string) {
    setProblem(null);
    try {
      const found = await findSubject(url, setStatus);
      if (!work.current) return;
      remember();
      const { mask } = work.current;
      const context = maskContext();
      context.globalCompositeOperation = "copy";
      context.drawImage(maskCanvas(found), 0, 0, mask.width, mask.height);
      context.globalCompositeOperation = "source-over";
    } catch (e) {
      if (!work.current) return;
      // Nothing is kept, so that a loop drawn round the subject keeps just that.
      remember();
      setAll(false);
      setMode("keep");
      setTool("loop");
      setProblem(`The subject could not be found automatically (${reason(e)}). Draw round what you want to keep instead.`);
    }
    setStatus(null);
    redraw();
  }

  // Loads the picture and makes the first attempt.
  useEffect(() => {
    if (!image) return;
    let stale = false;
    void (async () => {
      const picture = new Image();
      picture.src = image.url;
      await picture.decode();
      if (stale || !display.current) return;
      const scale = Math.min(1, WORK_MAX_PX / Math.max(picture.naturalWidth, picture.naturalHeight));
      const width = Math.max(1, Math.round(picture.naturalWidth * scale));
      const height = Math.max(1, Math.round(picture.naturalHeight * scale));
      const blank = () => Object.assign(document.createElement("canvas"), { width, height });
      display.current.width = width;
      display.current.height = height;
      work.current = { picture, mask: blank(), scratch: blank() };
      setAll(true);
      redraw();
      await find(image.url);
      history.current = [];
      setCanUndo(false);
    })().catch((e) => {
      setStatus(null);
      setProblem(reason(e));
    });
    return () => {
      stale = true;
      work.current = null;
    };
    // The picture cannot change while this is open: it covers the rest of the app.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [image?.url]);

  // Escape closes; Ctrl+Z takes back the last stroke here, not the last change to the project.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "z") {
        e.preventDefault();
        e.stopPropagation();
        undo();
      }
    };
    // Holding Space turns a drag into moving the view, as in drawing programs.
    const onSpace = (e: KeyboardEvent) => {
      if (e.code !== "Space" || (e.target as HTMLElement | null)?.tagName === "INPUT") return;
      spaceHeld.current = e.type === "keydown";
      if (e.type === "keydown") e.preventDefault();
    };
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("keydown", onSpace);
    window.addEventListener("keyup", onSpace);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("keydown", onSpace);
      window.removeEventListener("keyup", onSpace);
    };
  });

  // The size at which the whole picture fits, kept up as the window changes.
  useEffect(() => {
    const view = viewport.current;
    if (!view) return;
    const measure = () => {
      const canvas = display.current;
      if (!canvas || !work.current) return;
      const scale = Math.min((view.clientWidth - MARGIN * 2) / canvas.width, (view.clientHeight - MARGIN * 2) / canvas.height);
      setFit({ width: Math.max(1, canvas.width * scale), height: Math.max(1, canvas.height * scale) });
    };
    measure();
    const watcher = new ResizeObserver(measure);
    watcher.observe(view);
    return () => watcher.disconnect();
    // Measured again once the picture has loaded, which `status` first changing marks.
  }, [status === "Starting…"]);

  // The wheel zooms about the pointer. Listened to directly, since it must stop the view scrolling.
  useEffect(() => {
    const view = viewport.current;
    if (!view) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      zoomBy(e.deltaY < 0 ? ZOOM_STEP : 1 / ZOOM_STEP, e);
    };
    view.addEventListener("wheel", onWheel, { passive: false });
    return () => view.removeEventListener("wheel", onWheel);
  });

  // After a zoom, the view is moved so the point that was under the pointer still is.
  useLayoutEffect(() => {
    const view = viewport.current;
    const canvas = display.current;
    const held = anchor.current;
    anchor.current = null;
    if (!view || !canvas || !held) return;
    view.scrollLeft = canvas.offsetLeft + held.fx * canvas.offsetWidth - held.x;
    view.scrollTop = canvas.offsetTop + held.fy * canvas.offsetHeight - held.y;
  }, [zoom]);

  function draw() {
    const canvas = display.current;
    const current = work.current;
    if (!canvas || !current) return;
    const { width, height } = canvas;
    const context = canvas.getContext("2d")!;
    const scratch = current.scratch.getContext("2d")!;
    context.globalCompositeOperation = "source-over";
    if (view === "result") {
      scratch.globalCompositeOperation = "copy";
      scratch.drawImage(current.picture, 0, 0, width, height);
      scratch.globalCompositeOperation = "destination-in";
      scratch.drawImage(current.mask, 0, 0);
      context.fillStyle = backdrop === "white" ? "#ffffff" : "#000000";
      context.fillRect(0, 0, width, height);
    } else {
      context.drawImage(current.picture, 0, 0, width, height);
      scratch.globalCompositeOperation = "copy";
      scratch.fillStyle = WASH;
      scratch.fillRect(0, 0, width, height);
      scratch.globalCompositeOperation = "destination-out";
      scratch.drawImage(current.mask, 0, 0);
    }
    context.drawImage(current.scratch, 0, 0);
    if (view !== "edit") return;

    // About two screen pixels wide, however far in the view is.
    const line = (2 * width) / ((fit?.width ?? width) * zoom);
    context.lineWidth = line;
    context.strokeStyle = mode === "keep" ? "#ffffff" : "#ff5a36";
    const points = stroke.current;
    if (tool === "loop" && points && points.length > 1) {
      context.setLineDash([line * 4, line * 3]);
      context.beginPath();
      points.forEach(([x, y], i) => (i ? context.lineTo(x, y) : context.moveTo(x, y)));
      context.stroke();
      context.setLineDash([]);
    }
    if (tool === "brush" && pointer.current) {
      context.beginPath();
      context.arc(pointer.current[0], pointer.current[1], radius(), 0, Math.PI * 2);
      context.stroke();
    }
  }
  useEffect(draw);

  /** The brush's radius in the working copy's pixels. */
  const radius = () => {
    const canvas = display.current!;
    return ((brush / 100) * Math.max(canvas.width, canvas.height)) / 2;
  };

  const at = (e: PointerEvent<HTMLCanvasElement>): Point => {
    const canvas = e.currentTarget;
    const box = canvas.getBoundingClientRect();
    return [((e.clientX - box.left) * canvas.width) / box.width, ((e.clientY - box.top) * canvas.height) / box.height];
  };

  /** Applies a shape to the mask: kept where it is drawn, or removed. */
  function mark(shape: (context: CanvasRenderingContext2D) => void) {
    const context = maskContext();
    context.globalCompositeOperation = mode === "keep" ? "source-over" : "destination-out";
    context.fillStyle = context.strokeStyle = "#fff";
    shape(context);
    context.globalCompositeOperation = "source-over";
  }

  function brushStroke(from: Point, to: Point) {
    const r = radius();
    mark((context) => {
      context.lineWidth = r * 2;
      context.lineCap = context.lineJoin = "round";
      context.beginPath();
      context.moveTo(from[0], from[1]);
      context.lineTo(to[0], to[1]);
      context.stroke();
      context.beginPath();
      context.arc(to[0], to[1], r, 0, Math.PI * 2);
      context.fill();
    });
  }

  function onPointerDown(e: PointerEvent<HTMLCanvasElement>) {
    // The middle or right button, or the left with Space held, moves the view.
    if (e.button !== 0 || spaceHeld.current) {
      const frame = viewport.current!;
      e.currentTarget.setPointerCapture(e.pointerId);
      pan.current = { x: e.clientX, y: e.clientY, left: frame.scrollLeft, top: frame.scrollTop };
      e.preventDefault();
      return;
    }
    if (!work.current || status || view !== "edit") return;
    e.currentTarget.setPointerCapture(e.pointerId);
    const point = at(e);
    stroke.current = [point];
    if (tool === "brush") {
      remember();
      brushStroke(point, point);
    }
    draw();
  }

  function onPointerMove(e: PointerEvent<HTMLCanvasElement>) {
    if (pan.current) {
      const frame = viewport.current!;
      frame.scrollLeft = pan.current.left - (e.clientX - pan.current.x);
      frame.scrollTop = pan.current.top - (e.clientY - pan.current.y);
      return;
    }
    const point = at(e);
    pointer.current = point;
    const points = stroke.current;
    if (points) {
      if (tool === "brush") {
        brushStroke(points[points.length - 1], point);
        stroke.current = [point];
      } else {
        points.push(point);
      }
    }
    draw();
  }

  function onPointerUp() {
    if (pan.current) {
      pan.current = null;
      return;
    }
    const points = stroke.current;
    stroke.current = null;
    // A loop closes itself: its end is joined to its start.
    if (tool === "loop" && points && points.length > 2) {
      remember();
      mark((context) => {
        context.beginPath();
        points.forEach(([x, y], i) => (i ? context.lineTo(x, y) : context.moveTo(x, y)));
        context.closePath();
        context.fill();
      });
    }
    draw();
  }

  async function apply() {
    const current = work.current;
    if (!current || !image) return;
    setWorking(true);
    setProblem(null);
    try {
      const { mask } = current;
      const alpha = maskContext().getImageData(0, 0, mask.width, mask.height).data;
      let kept = false;
      for (let i = 3; i < alpha.length && !kept; i += 4) kept = alpha[i] > 127;
      if (!kept) throw new Error("Nothing is kept. Draw round what you want to keep, or press Keep everything and start again.");
      await useProject.getState().replacePicture(await cutOut(image.url, mask, backdrop));
      close();
    } catch (e) {
      setProblem(reason(e));
    } finally {
      setWorking(false);
    }
  }

  /** Runs a change to the whole mask that can be undone. */
  const whole = (change: () => void) => () => {
    if (!work.current) return;
    remember();
    change();
    redraw();
  };

  if (!image) return null;
  const busy = status !== null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-6" onClick={close}>
      <div
        role="dialog"
        aria-label="Remove the background"
        className="flex h-full max-h-[860px] w-full max-w-[1120px] flex-col overflow-hidden rounded-2xl border border-line bg-surface shadow-xl"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="flex items-center border-b border-line px-5 py-3">
          <h2 className="text-[14px] font-semibold">Remove the background</h2>
          <span className="ml-3 text-muted">Blue marks what will be removed</span>
          <button onClick={close} aria-label="Close remove the background" className="ml-auto rounded p-1 text-muted hover:text-fg">
            <X size={16} />
          </button>
        </header>

        <div className="flex min-h-0 flex-1">
          <div className="relative min-h-0 min-w-0 flex-1 bg-raised">
            {/* The picture's own window: it scrolls when the picture is magnified past its edges. */}
            <div ref={viewport} className="absolute inset-0 flex overflow-auto" onContextMenu={(e) => e.preventDefault()}>
            <canvas
              ref={display}
              aria-label="The picture, to draw on"
              className="m-auto shrink-0 touch-none rounded shadow"
              style={{
                cursor: view === "edit" && !busy ? "crosshair" : "default",
                width: fit ? fit.width * zoom : undefined,
                height: fit ? fit.height * zoom : undefined,
                visibility: fit ? "visible" : "hidden",
              }}
              onPointerDown={onPointerDown}
              onPointerMove={onPointerMove}
              onPointerUp={onPointerUp}
              onPointerCancel={onPointerUp}
              onPointerLeave={() => {
                pointer.current = null;
                draw();
              }}
            />
            </div>
            <div className="absolute bottom-3 left-3 flex items-center gap-1 rounded-lg border border-line bg-surface p-1 shadow">
              <button className="rounded p-1.5 text-muted hover:text-fg disabled:opacity-40" disabled={zoom <= 1} aria-label="Zoom out" title="Zoom out" onClick={() => zoomBy(1 / ZOOM_STEP)}>
                <ZoomOut size={15} />
              </button>
              <span className="w-12 text-center font-mono text-[11px] text-muted" aria-label="Zoom">
                {Math.round(zoom * 100)}%
              </span>
              <button className="rounded p-1.5 text-muted hover:text-fg disabled:opacity-40" disabled={zoom >= ZOOM_MAX} aria-label="Zoom in" title="Zoom in, for fine work" onClick={() => zoomBy(ZOOM_STEP)}>
                <ZoomIn size={15} />
              </button>
              <button className="rounded p-1.5 text-muted hover:text-fg disabled:opacity-40" disabled={zoom === 1} aria-label="Show the whole picture" title="Show the whole picture" onClick={() => setZoom(1)}>
                <Maximize size={15} />
              </button>
            </div>
            {busy && (
              <div className="absolute inset-0 flex items-center justify-center bg-black/35">
                <p role="status" className="rounded-lg border border-line bg-surface px-4 py-2 shadow">
                  {status}
                </p>
              </div>
            )}
          </div>

          <div className="flex w-[300px] shrink-0 flex-col gap-4 overflow-y-auto border-l border-line p-5">
            <p className="text-muted">
              The subject is found for you. Where it has it wrong, draw on the picture to put it right: a loop
              round an area, or a brush over it.
            </p>
            <p className="text-muted">
              Turn the mouse wheel to zoom in for fine work. To move around, hold Space and drag, or drag with
              the right mouse button.
            </p>

            <div className="flex flex-col gap-1.5">
              <span className={caption}>What you draw is</span>
              <Segmented
                label="What you draw is"
                value={mode}
                options={[
                  { value: "keep", label: "Kept" },
                  { value: "remove", label: "Removed" },
                ]}
                onChange={setMode}
              />
            </div>
            <div className="flex flex-col gap-1.5">
              <span className={caption}>Draw with</span>
              <Segmented
                label="Draw with"
                value={tool}
                options={[
                  { value: "loop", label: "A loop round it" },
                  { value: "brush", label: "A brush" },
                ]}
                onChange={setTool}
              />
            </div>
            {tool === "brush" && (
              <Slider
                label="Brush size"
                unit="%"
                min={0.5}
                max={20}
                step={0.5}
                value={brush}
                hint={{ text: "How wide the brush is, as a share of the picture. Use a small brush along edges and a large one to fill in." }}
                onChange={setBrush}
              />
            )}

            <div className="flex flex-wrap gap-2">
              <button className={quiet} disabled={!canUndo || busy} onClick={undo} title="Take back the last thing drawn (Ctrl+Z)">
                Undo
              </button>
              <button
                className={quiet}
                disabled={busy}
                title="Throw away the corrections and find the subject afresh"
                onClick={() => {
                  setStatus("Finding the subject…");
                  void find(image.url);
                }}
              >
                Find the subject again
              </button>
              <button
                className={quiet}
                disabled={busy}
                title="Remove the whole picture, then draw round just what you want to keep"
                onClick={whole(() => {
                  setAll(false);
                  setMode("keep");
                })}
              >
                Remove everything
              </button>
              <button
                className={quiet}
                disabled={busy}
                title="Keep the whole picture, then draw round what you want removed"
                onClick={whole(() => {
                  setAll(true);
                  setMode("remove");
                })}
              >
                Keep everything
              </button>
            </div>

            <div className="flex flex-col gap-1.5">
              <span className={caption}>Show</span>
              <Segmented
                label="Show"
                value={view}
                options={[
                  { value: "edit", label: "Marked up" },
                  { value: "result", label: "Finished" },
                ]}
                onChange={setView}
              />
            </div>
            <Select
              label="Background becomes"
              value={backdrop}
              options={[
                { value: "white", label: "White" },
                { value: "black", label: "Black" },
              ]}
              hint={{
                text: "What is painted where the background was. White is left alone by a laser and is not traced, so it suits nearly every job. Choose black for a carving where dark is low and the subject should stand up from a flat ground.",
              }}
              onChange={setBackdrop}
            />

            {problem && (
              <p role="alert" className="rounded-lg border border-accent/40 px-3 py-2 text-accent">
                {problem}
              </p>
            )}
          </div>
        </div>

        <footer className="flex items-center justify-end gap-2 border-t border-line px-5 py-3">
          <span className="mr-auto text-muted">Restore the picture, in Design, brings the background back.</span>
          <button onClick={close} title="Close and leave the picture as it is" className={`${quiet} px-4 py-2`}>
            Cancel
          </button>
          <button
            onClick={() => void apply()}
            disabled={working || busy}
            title="Use the picture without its background"
            className={primary}
          >
            {working ? "Working…" : "Remove the background"}
          </button>
        </footer>
      </div>
    </div>
  );
}
