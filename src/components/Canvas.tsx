import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { Upload } from "lucide-react";
import { useProject } from "../state/project";

/** Top-left corner of the visible area in mm, and the zoom as mm per screen pixel. */
interface View {
  x: number;
  y: number;
  mmPerPx: number;
}

const MARGIN_MM = 30;
const MIN_MM_PER_PX = 0.01;
const MAX_MM_PER_PX = 5;

export function Canvas() {
  const bed = useProject((s) => s.bed);
  const image = useProject((s) => s.image);
  const importFile = useProject((s) => s.importFile);
  const widthMm = useProject((s) => s.widthMm);
  const showSource = useProject((s) => s.showSource);
  const traced = useProject((s) => s.trace.result);
  const traceMode = useProject((s) => s.trace.settings.mode);

  const containerRef = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ w: 0, h: 0 });
  const [view, setView] = useState<View | null>(null);
  const [dragOver, setDragOver] = useState(false);
  const pan = useRef<{ x: number; y: number } | null>(null);

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

  // Fit once, when the container first has a size.
  useEffect(() => {
    if (!view) fit();
  }, [view, fit]);

  function onWheel(e: React.WheelEvent) {
    if (!view) return;
    const rect = containerRef.current!.getBoundingClientRect();
    const px = e.clientX - rect.left;
    const py = e.clientY - rect.top;
    const mmPerPx = Math.min(
      MAX_MM_PER_PX,
      Math.max(MIN_MM_PER_PX, view.mmPerPx * Math.exp(e.deltaY * 0.0015)),
    );
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

  const placed = image && placeImage(image, widthMm, bed);
  const px = view?.mmPerPx ?? 1;

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
          {image && placed && (showSource || !traced) && (
            <image
              href={image.url}
              x={placed.x}
              y={placed.y}
              width={placed.width}
              height={placed.height}
              preserveAspectRatio="none"
              opacity={traced ? 0.25 : 1}
            />
          )}
          {traced && placed && (
            // The markup is generated by our own tracer from numeric path data.
            <svg
              x={placed.x}
              y={placed.y}
              width={placed.width}
              height={placed.height}
              viewBox={`0 0 ${traced.width} ${traced.height}`}
              preserveAspectRatio="none"
              className={traceMode === "outline" ? "trace-mono" : undefined}
              dangerouslySetInnerHTML={{ __html: traced.body }}
            />
          )}
        </svg>
      )}

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

      <div className="pointer-events-none absolute bottom-3 left-3 rounded-md border border-line bg-surface/90 px-2 py-1 font-mono text-[11px] text-muted">
        {bed.width} × {bed.height} mm
        {view && ` · ${(100 / (view.mmPerPx * 3.7795)).toFixed(0)}%`}
      </div>
    </main>
  );
}

/** Centres the artwork on the bed at its physical size. */
function placeImage(
  image: { width: number; height: number },
  widthMm: number,
  bed: { width: number; height: number },
) {
  const height = (widthMm * image.height) / image.width;
  return { x: (bed.width - widthMm) / 2, y: (bed.height - height) / 2, width: widthMm, height };
}
