import { useEffect, useMemo, useRef, useState } from "react";
import { Camera, TriangleAlert, X } from "lucide-react";
import { checkCameraFit, cameraError, type CameraFit, type FitPoint } from "../lib/camera";
import { useCamera } from "../state/camera";
import { useProject } from "../state/project";

type Corner = "backLeft" | "backRight" | "frontRight" | "frontLeft";
const CORNERS: { id: Corner; name: string }[] = [
  { id: "backLeft", name: "Back left" },
  { id: "backRight", name: "Back right" },
  { id: "frontRight", name: "Front right" },
  { id: "frontLeft", name: "Front left" },
];

/** A marker as the person sees it: where in the picture, and its machine X and Y (from the origin corner). */
interface Marker {
  px: number;
  py: number;
  machineX: number;
  machineY: number;
}

type Bed = { width: number; height: number };
type Origin = "frontLeft" | "backLeft";

/** Machine X/Y (measured from the origin corner) to canvas mm (Y down from the back), as `itemRect` does. */
const toCanvas = (m: Marker, bed: Bed, origin: Origin) => ({ x: m.machineX, y: origin === "backLeft" ? m.machineY : bed.height - m.machineY });
const fromCanvas = (x: number, y: number, bed: Bed, origin: Origin) => ({ machineX: x, machineY: origin === "backLeft" ? y : bed.height - y });

function cornerCanvas(corner: Corner, bed: Bed) {
  return {
    x: corner === "backLeft" || corner === "frontLeft" ? 0 : bed.width,
    y: corner === "backLeft" || corner === "backRight" ? 0 : bed.height,
  };
}

/**
 * A bed-mm → picture-pixel mapping, only for drawing the check grid on
 * screen. The engine does the real redrawing with its own copy.
 */
function bedToPixel(points: FitPoint[]): ((x: number, y: number) => [number, number] | null) | null {
  const a: number[][] = [];
  for (const p of points) {
    a.push([p.x, p.y, 1, 0, 0, 0, -p.px * p.x, -p.px * p.y, p.px]);
    a.push([0, 0, 0, p.x, p.y, 1, -p.py * p.x, -p.py * p.y, p.py]);
  }
  for (let col = 0; col < 8; col++) {
    let pivot = col;
    for (let r = col + 1; r < 8; r++) if (Math.abs(a[r][col]) > Math.abs(a[pivot][col])) pivot = r;
    if (Math.abs(a[pivot][col]) < 1e-12) return null;
    [a[col], a[pivot]] = [a[pivot], a[col]];
    for (let r = 0; r < 8; r++) {
      if (r === col) continue;
      const f = a[r][col] / a[col][col];
      for (let k = col; k < 9; k++) a[r][k] -= f * a[col][k];
    }
  }
  const h = a.map((row, i) => row[8] / row[i]);
  return (x, y) => {
    const w = h[6] * x + h[7] * y + 1;
    if (!(w > 1e-9)) return null;
    return [(h[0] * x + h[1] * y + h[2]) / w, (h[3] * x + h[4] * y + h[5]) / w];
  };
}

function defaultMarkers(width: number, height: number, bed: Bed, origin: Origin): Marker[] {
  const inset = [
    [0.2, 0.2],
    [0.8, 0.2],
    [0.8, 0.8],
    [0.2, 0.8],
  ];
  return CORNERS.map((c, i) => {
    const at = cornerCanvas(c.id, bed);
    return { px: inset[i][0] * width, py: inset[i][1] * height, ...fromCanvas(at.x, at.y, bed, origin) };
  });
}

/** Lining the camera up with the bed: four markers on four known places. */
export function CameraLineUp() {
  const open = useCamera((s) => s.lineUpOpen);
  const still = useCamera((s) => s.still);
  const fit = useCamera((s) => s.fit);
  const busy = useCamera((s) => s.busy);
  const camera = useCamera.getState();
  const bed = useProject((s) => s.bed);
  const origin = useProject((s) => s.origin);

  const [markers, setMarkers] = useState<Marker[]>([]);
  const [checking, setChecking] = useState(true);
  const [problem, setProblem] = useState<string | null>(null);
  const [mirrored, setMirrored] = useState(false);
  const dragging = useRef<number | null>(null);
  const svg = useRef<SVGSVGElement>(null);

  // Start from the saved markers when they fit this picture and bed, otherwise near the corners.
  useEffect(() => {
    if (!open || !still) return;
    const usable = fit && fit.pictureWidth === still.width && fit.pictureHeight === still.height && fit.bedWidth === bed.width && fit.bedHeight === bed.height;
    setMarkers(
      usable
        ? fit.points.map((p) => ({ px: p.px, py: p.py, ...fromCanvas(p.x, p.y, bed, origin) }))
        : defaultMarkers(still.width, still.height, bed, origin),
    );
    // Only when opening or a new picture arrives.
  }, [open, still]);

  const points: FitPoint[] = useMemo(
    () => markers.map((m) => ({ px: m.px, py: m.py, ...toCanvas(m, bed, origin) })),
    [markers, bed, origin],
  );

  const candidate: CameraFit | null =
    still && points.length === 4
      ? {
          points: points as CameraFit["points"],
          bedWidth: bed.width,
          bedHeight: bed.height,
          pictureWidth: still.width,
          pictureHeight: still.height,
        }
      : null;

  // The engine says whether the markers make a usable fit, and if it looks mirrored.
  useEffect(() => {
    if (!candidate) return;
    let current = true;
    const timer = window.setTimeout(() => {
      checkCameraFit(candidate)
        .then((check) => current && (setProblem(null), setMirrored(check.mirrored)))
        .catch((e) => current && (setProblem(cameraError(e)), setMirrored(false)));
    }, 150);
    return () => {
      current = false;
      window.clearTimeout(timer);
    };
  }, [JSON.stringify(candidate)]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && camera.openLineUp(false);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open]);

  if (!open) return null;

  const close = () => camera.openLineUp(false);
  const map = !problem && points.length === 4 ? bedToPixel(points) : null;
  const scale = still ? Math.max(still.width, still.height) / 900 : 1;

  function pictureAt(e: React.PointerEvent): [number, number] | null {
    const el = svg.current;
    const ctm = el?.getScreenCTM();
    if (!el || !ctm || !still) return null;
    const p = new DOMPoint(e.clientX, e.clientY).matrixTransform(ctm.inverse());
    return [Math.min(still.width, Math.max(0, p.x)), Math.min(still.height, Math.max(0, p.y))];
  }

  const setMarker = (i: number, patch: Partial<Marker>) => setMarkers((ms) => ms.map((m, j) => (j === i ? { ...m, ...patch } : m)));

  /** A bed line drawn through the fit, as a polyline in picture pixels. */
  function line(from: [number, number], to: [number, number]): string {
    if (!map) return "";
    const out: string[] = [];
    for (let i = 0; i <= 24; i++) {
      const t = i / 24;
      const p = map(from[0] + (to[0] - from[0]) * t, from[1] + (to[1] - from[1]) * t);
      if (p) out.push(`${p[0].toFixed(1)},${p[1].toFixed(1)}`);
    }
    return out.join(" ");
  }

  const gridStep = Math.max(bed.width, bed.height) > 1000 ? 100 : 50;
  const zero = origin === "backLeft" ? { x: 0, y: 0 } : { x: 0, y: bed.height };
  const yDir = origin === "backLeft" ? 1 : -1;
  const arrow = Math.min(bed.width, bed.height) * 0.2;
  const zeroAt = map?.(zero.x, zero.y);
  const xTip = map?.(zero.x + arrow, zero.y);
  const yTip = map?.(zero.x, zero.y + yDir * arrow);

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-6" onClick={close}>
      <div
        role="dialog"
        aria-label="Line up the camera"
        className="flex h-full max-h-[860px] w-full max-w-[1200px] flex-col overflow-hidden rounded-2xl border border-line bg-surface shadow-xl"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="flex items-center border-b border-line px-5 py-3">
          <h2 className="text-[14px] font-semibold">Line up the camera</h2>
          <span className="ml-3 text-muted">Drag each marker onto the place it names</span>
          <button onClick={close} aria-label="Close" className="ml-auto rounded p-1 text-muted hover:text-fg">
            <X size={16} />
          </button>
        </header>

        <div className="flex min-h-0 flex-1">
          <div className="relative min-w-0 flex-1 bg-bg p-3">
            {still ? (
              <svg
                ref={svg}
                className="h-full w-full touch-none select-none"
                viewBox={`0 0 ${still.width} ${still.height}`}
                preserveAspectRatio="xMidYMid meet"
                onPointerMove={(e) => {
                  if (dragging.current === null) return;
                  const at = pictureAt(e);
                  if (at) setMarker(dragging.current, { px: at[0], py: at[1] });
                }}
                onPointerUp={() => (dragging.current = null)}
                onPointerLeave={() => (dragging.current = null)}
              >
                <image href={still.url} width={still.width} height={still.height} />
                {checking && map && (
                  <g fill="none" strokeWidth={1.5 * scale}>
                    {Array.from({ length: Math.floor(bed.width / gridStep) + 1 }, (_, i) => i * gridStep).map((x) => (
                      <polyline key={`x${x}`} points={line([x, 0], [x, bed.height])} stroke="#3fd0ff" opacity={0.8} />
                    ))}
                    {Array.from({ length: Math.floor(bed.height / gridStep) + 1 }, (_, i) => i * gridStep).map((y) => (
                      <polyline key={`y${y}`} points={line([0, y], [bed.width, y])} stroke="#3fd0ff" opacity={0.8} />
                    ))}
                    <polyline
                      points={[line([0, 0], [bed.width, 0]), line([bed.width, 0], [bed.width, bed.height]), line([bed.width, bed.height], [0, bed.height]), line([0, bed.height], [0, 0])].join(" ")}
                      stroke="#3fd0ff"
                      strokeWidth={3 * scale}
                    />
                    {/* Which way is which: a symmetric grid would hide a mirrored or turned fit. */}
                    {zeroAt && xTip && (
                      <line x1={zeroAt[0]} y1={zeroAt[1]} x2={xTip[0]} y2={xTip[1]} stroke="#ff4d4d" strokeWidth={4 * scale} />
                    )}
                    {zeroAt && yTip && (
                      <line x1={zeroAt[0]} y1={zeroAt[1]} x2={yTip[0]} y2={yTip[1]} stroke="#3ddc68" strokeWidth={4 * scale} />
                    )}
                    {xTip && (
                      <text x={xTip[0]} y={xTip[1]} fill="#ff4d4d" stroke="black" strokeWidth={0.6 * scale} fontSize={22 * scale} fontWeight="bold">X</text>
                    )}
                    {yTip && (
                      <text x={yTip[0]} y={yTip[1]} fill="#3ddc68" stroke="black" strokeWidth={0.6 * scale} fontSize={22 * scale} fontWeight="bold">Y</text>
                    )}
                    {zeroAt && (
                      <text x={zeroAt[0] + 6 * scale} y={zeroAt[1] - 6 * scale} fill="white" stroke="black" strokeWidth={0.6 * scale} fontSize={20 * scale} fontWeight="bold">0,0</text>
                    )}
                  </g>
                )}
                {markers.map((m, i) => (
                  <g key={i} style={{ cursor: "grab" }} onPointerDown={(e) => {
                    e.preventDefault();
                    (e.target as Element).setPointerCapture?.(e.pointerId);
                    dragging.current = i;
                  }}>
                    <circle cx={m.px} cy={m.py} r={18 * scale} fill="rgba(255,255,255,0.15)" stroke="white" strokeWidth={2 * scale} />
                    <circle cx={m.px} cy={m.py} r={18 * scale} fill="none" stroke="black" strokeWidth={0.8 * scale} />
                    <line x1={m.px - 26 * scale} x2={m.px + 26 * scale} y1={m.py} y2={m.py} stroke="#ff9d2e" strokeWidth={1.5 * scale} />
                    <line x1={m.px} x2={m.px} y1={m.py - 26 * scale} y2={m.py + 26 * scale} stroke="#ff9d2e" strokeWidth={1.5 * scale} />
                    <text x={m.px + 22 * scale} y={m.py - 20 * scale} fill="white" stroke="black" strokeWidth={0.6 * scale} fontSize={20 * scale} fontWeight="bold">
                      {i + 1} {CORNERS[i].name}
                    </text>
                  </g>
                ))}
              </svg>
            ) : (
              <div className="flex h-full items-center justify-center text-muted">Take a picture of the bed first.</div>
            )}
          </div>

          <aside className="flex w-80 shrink-0 flex-col gap-4 overflow-y-auto border-l border-line p-4">
            <p className="text-muted">
              Drag each numbered marker onto that place in the picture. The bed's corners are best. If a corner can't be seen,
              use another place you can measure, such as the corner of a sheet pushed into the origin corner, and type where it
              is on the machine.
            </p>
            {markers.map((m, i) => (
              <div key={i} className="flex flex-col gap-1">
                <span className="font-medium">
                  {i + 1}. {CORNERS[i].name}
                </span>
                <div className="flex items-center gap-2 text-muted">
                  <label className="flex items-center gap-1">
                    X
                    <input
                      type="number"
                      className="w-20 rounded-md border border-line bg-raised px-1.5 py-0.5 text-right font-mono text-[12px]"
                      value={Number(m.machineX.toFixed(1))}
                      onChange={(e) => Number.isFinite(e.target.valueAsNumber) && setMarker(i, { machineX: e.target.valueAsNumber })}
                    />
                  </label>
                  <label className="flex items-center gap-1">
                    Y
                    <input
                      type="number"
                      className="w-20 rounded-md border border-line bg-raised px-1.5 py-0.5 text-right font-mono text-[12px]"
                      value={Number(m.machineY.toFixed(1))}
                      onChange={(e) => Number.isFinite(e.target.valueAsNumber) && setMarker(i, { machineY: e.target.valueAsNumber })}
                    />
                  </label>
                  <span>mm</span>
                </div>
              </div>
            ))}
            <p className="text-[12px] text-muted">X and Y are measured from the machine's X0 Y0 corner ({origin === "backLeft" ? "back left" : "front left"}).</p>

            <label className="flex items-center gap-2">
              <input type="checkbox" checked={checking} onChange={(e) => setChecking(e.target.checked)} />
              Check: draw the bed's grid over the picture
            </label>
            <p className="text-[12px] text-muted">
              The blue lines should sit on the real bed. The red arrow should point along X and the green one along Y, from the 0,0 corner.
            </p>

            {problem && (
              <p role="alert" className="flex gap-2 rounded-lg border border-accent/40 px-3 py-2">
                <TriangleAlert size={15} className="mt-0.5 shrink-0 text-accent" />
                {problem}
              </p>
            )}
            {mirrored && !problem && (
              <p role="alert" className="flex gap-2 rounded-lg border border-accent/40 px-3 py-2">
                <TriangleAlert size={15} className="mt-0.5 shrink-0 text-accent" />
                The picture looks mirrored: the markers go round the other way from the bed's corners. Check each marker is on the
                corner it is named for. A camera mounted upside down can make left and right easy to swap.
              </p>
            )}

            <div className="mt-auto flex flex-col gap-2">
              <button
                className="flex items-center justify-center gap-2 rounded-lg border border-line px-3 py-2 hover:border-muted disabled:opacity-50"
                disabled={busy}
                onClick={() => void camera.capture()}
              >
                <Camera size={15} /> Take a new picture
              </button>
              <button
                className="rounded-lg bg-accent px-3 py-2 font-medium text-accent-fg transition-opacity hover:opacity-90 disabled:opacity-50"
                disabled={!candidate || !!problem}
                onClick={() => candidate && camera.saveFit(candidate)}
              >
                Use this lining up
              </button>
            </div>
          </aside>
        </div>
      </div>
    </div>
  );
}
