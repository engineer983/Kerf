import { useEffect } from "react";
import { ScanSearch, TriangleAlert } from "lucide-react";
import { NOT_LINED_UP_MESSAGE, closeToTyped, notLinedUp, outlinePlan, useCamera } from "../state/camera";
import { everythingRect, stockRect, useProject } from "../state/project";
import { useUi } from "../state/ui";

const round = (mm: number) => Math.round(mm);
const quiet = "rounded-lg border border-line px-3 py-1 transition-colors hover:text-accent disabled:opacity-40";
const primary = "rounded-lg bg-accent px-3 py-1 font-medium text-accent-fg transition-opacity hover:opacity-90";

/** Why "Find the material" can't be used yet, or null when it can. */
function useNotReady(): string | null {
  const fit = useCamera((s) => s.fit);
  const fitDeviceId = useCamera((s) => s.fitDeviceId);
  const deviceId = useCamera((s) => s.deviceId);
  const bed = useProject((s) => s.bed);
  if (!deviceId && !fit) return "Set up a camera first, in step 1, Device.";
  const reason = notLinedUp({ fit, fitDeviceId, deviceId, still: null }, bed);
  return reason ? NOT_LINED_UP_MESSAGE[reason] : null;
}

/**
 * Looks for the material in the camera picture. Greyed out, with the
 * reason, until the camera has been lined up. Never applies anything itself.
 */
export function FindMaterialButton({ className, showReason = false }: { className: string; showReason?: boolean }) {
  const notReady = useNotReady();
  const busy = useCamera((s) => s.busy || s.finding);
  return (
    <>
      <button
        className={className}
        disabled={notReady !== null || busy}
        title={notReady ?? "Look for the material in a camera picture of the bed, then check the outline before using it"}
        onClick={() => void useCamera.getState().findMaterial()}
      >
        <span className="inline-flex items-center gap-1.5">
          <ScanSearch size={14} /> {busy ? "Looking…" : "Find the material"}
        </span>
      </button>
      {showReason && notReady && <p className="w-full text-[12px] text-muted">{notReady}</p>}
    </>
  );
}

/**
 * The found outline over the camera picture, in bed millimetres: its corner
 * markers while it is being checked, and a click target while waiting for a
 * click on the material. Once used, the outline stays as a guide for as long
 * as the picture is shown.
 */
export function FoundOverlay({
  viewBox,
  px,
  toBed,
}: {
  viewBox: string;
  px: number;
  toBed: (e: { clientX: number; clientY: number }) => [number, number];
}) {
  const found = useCamera((s) => s.found);
  const confirming = useCamera((s) => s.confirming);
  const picking = useCamera((s) => s.picking);
  const shown = useCamera((s) => s.show && s.bedPictureUrl !== null);
  const keepTypedSize = useCamera((s) => s.keepTypedSize);
  const bed = useProject((s) => s.bed);
  const stockWidth = useProject((s) => s.stockWidth);
  const stockLength = useProject((s) => s.stockLength);
  const camera = useCamera.getState();
  if (!picking && !(found && (confirming || shown))) return null;

  const outline = found?.corners.map((c) => c.join(",")).join(" ");
  const plan = found && confirming ? outlinePlan(found, keepTypedSize, { stockWidth, stockLength }) : null;
  return (
    <svg className="pointer-events-none absolute inset-0 h-full w-full" viewBox={viewBox}>
      {picking && (
        <rect
          width={bed.width}
          height={bed.height}
          fill="transparent"
          style={{ pointerEvents: "auto", cursor: "crosshair" }}
          onPointerDown={(e) => {
            if (e.button !== 0) return;
            e.stopPropagation();
            void camera.findMaterial(toBed(e));
          }}
          onDoubleClick={(e) => e.stopPropagation()}
        />
      )}
      {found && outline && (
        <>
          <polygon points={outline} fill="none" stroke="black" strokeOpacity={0.5} strokeWidth={px * 3} />
          <polygon points={outline} fill="none" stroke="var(--accent)" strokeWidth={px * 2} strokeDasharray={`${px * 8} ${px * 5}`} />
        </>
      )}
      {plan?.turned && (
        // What would be used of a turned sheet: the straight part in the middle.
        <rect {...plan.rect} fill="var(--accent)" fillOpacity={0.12} stroke="var(--accent)" strokeWidth={px} />
      )}
      {found &&
        confirming &&
        !picking &&
        found.corners.map(([x, y], i) => (
          <circle
            key={i}
            cx={x}
            cy={y}
            r={px * 7}
            fill="var(--raised)"
            stroke="var(--accent)"
            strokeWidth={px * 2}
            style={{ pointerEvents: "auto", cursor: "grab" }}
            onPointerDown={(e) => {
              if (e.button !== 0) return;
              // Dragging a corner must not also pan the canvas underneath.
              e.stopPropagation();
              e.currentTarget.setPointerCapture(e.pointerId);
            }}
            onPointerMove={(e) => {
              if (e.currentTarget.hasPointerCapture(e.pointerId)) camera.setCorner(i, toBed(e));
            }}
            onDoubleClick={(e) => e.stopPropagation()}
          />
        ))}
    </svg>
  );
}

/** Checking a found outline before it is used, and the reminder to frame the job after. */
export function FoundPanel() {
  const found = useCamera((s) => s.found);
  const confirming = useCamera((s) => s.confirming);
  const finding = useCamera((s) => s.finding);
  const picking = useCamera((s) => s.picking);
  const findError = useCamera((s) => s.findError);
  const keepTypedSize = useCamera((s) => s.keepTypedSize);
  const framePrompt = useCamera((s) => s.framePrompt);
  const project = useProject();
  const camera = useCamera.getState();

  useEffect(() => {
    if (!confirming) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      if (useCamera.getState().picking) camera.setPicking(false);
      else camera.cancelFound();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [confirming]);

  // The note on the canvas is only for an outline just used; a typed move
  // gets the reminder in the Machine step instead.
  if (!confirming && framePrompt !== "found") return null;

  const box = (children: React.ReactNode) => (
    <div
      role="dialog"
      aria-label="Found material"
      className="absolute top-3 left-1/2 flex w-[min(30rem,calc(100%-1.5rem))] -translate-x-1/2 cursor-default flex-col gap-2 rounded-xl border border-line bg-surface/95 px-4 py-3 shadow-sm backdrop-blur"
      onPointerDown={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
      onWheel={(e) => e.stopPropagation()}
    >
      {children}
    </div>
  );

  if (!confirming) {
    const all = everythingRect(project);
    const stock = stockRect(project);
    const overhangs =
      all !== null &&
      (all.x < stock.x - 0.01 ||
        all.y < stock.y - 0.01 ||
        all.x + all.width > stock.x + stock.width + 0.01 ||
        all.y + all.height > stock.y + stock.height + 0.01);
    return box(
      <>
        <p>
          The material has been placed and the design centred on it. Ctrl+Z puts it back.{" "}
          <strong>Frame the job to check it sits on the material before you start.</strong>
        </p>
        {zeroWarning(project) && <p>{zeroWarning(project)}</p>}
        {overhangs && (
          <p role="alert" className="flex gap-1.5 text-accent">
            <TriangleAlert size={14} className="mt-0.5 shrink-0" /> The design reaches past the edge of the material.
          </p>
        )}
        <div className="flex flex-wrap gap-2">
          {overhangs && (
            <button className={quiet} onClick={project.fitEverythingOnMaterial}>
              Make it fit
            </button>
          )}
          <button className={quiet} onClick={camera.dismissFramePrompt}>
            OK
          </button>
        </div>
      </>,
    );
  }

  const cancel = (
    <button className={quiet} onClick={camera.cancelFound}>
      Cancel
    </button>
  );
  const clickToTry = (
    <button className={quiet} onClick={() => camera.setPicking(true)} title="Click on a plain part of the material in the picture, and Kerf looks again from there">
      Click on the material to try again
    </button>
  );

  if (picking) {
    return box(
      <>
        <p>Click on the material in the picture, on a plain part of it away from the edges.</p>
        <div className="flex gap-2">
          <button className={quiet} onClick={() => camera.setPicking(false)}>
            Back
          </button>
          {cancel}
        </div>
      </>,
    );
  }
  if (finding) return box(<p className="text-muted">Looking for the material…</p>);
  if (findError || !found) {
    return box(
      <>
        <p role="alert" className="flex gap-1.5">
          <TriangleAlert size={14} className="mt-0.5 shrink-0 text-accent" /> {findError ?? "The material wasn't found."}
        </p>
        <div className="flex flex-wrap gap-2">
          {clickToTry}
          {cancel}
        </div>
      </>,
    );
  }

  const plan = outlinePlan(found, keepTypedSize, project);
  const { measured } = plan;
  const angle = Math.abs(measured.angleDeg);
  const angleText = angle < 0.05 ? "square to the bed" : `at an angle of ${angle < 2 ? angle.toFixed(1) : angle.toFixed(0)}°`;
  const sameAsTyped = closeToTyped(measured, project);
  const doubtful = found.result.warnings.length > 0 || plan.turned;
  return box(
    <>
      <p className="font-medium">
        Material found: {round(measured.width)} × {round(measured.length)} mm, {angleText}
      </p>
      {found.result.warnings.length > 0 && (
        <ul className="flex flex-col gap-1">
          {found.result.warnings.map((w) => (
            <li key={w} className="flex gap-1.5">
              <TriangleAlert size={14} className="mt-0.5 shrink-0 text-accent" /> {w}
            </li>
          ))}
        </ul>
      )}
      {project.machineKind === "cnc" && (
        <p className="flex gap-1.5">
          <TriangleAlert size={14} className="mt-0.5 shrink-0 text-accent" /> Check the outline doesn't include a clamp. Keep the design at
          least 10 mm from any clamp.
        </p>
      )}
      <label className="flex items-center gap-2">
        <input type="checkbox" checked={keepTypedSize} onChange={(e) => camera.setKeepTypedSize(e.target.checked)} />
        Keep my size, {project.stockWidth} × {project.stockLength} mm
        {!sameAsTyped && <span className="text-muted">(the camera sees {round(measured.width)} × {round(measured.length)} mm)</span>}
      </label>
      {plan.turned && (
        <p>
          The material is at an angle, so Kerf will use the straight part in the middle: {round(plan.rect.width)} ×{" "}
          {round(plan.rect.height)} mm. Straighten the material for the full size.
        </p>
      )}
      <p className="text-[12px] text-muted">
        Drag the corners to correct the outline. The outline is a guess: check it, then frame the job before cutting.{" "}
        <button className="underline underline-offset-2 hover:text-fg" onClick={() => useUi.getState().setHelp("find-material")}>
          More about this
        </button>
      </p>
      <div className="flex flex-wrap gap-2">
        <button className={primary} onClick={camera.applyFound}>
          {doubtful ? "Use it anyway" : "Use this outline"}
        </button>
        {clickToTry}
        {cancel}
      </div>
    </>,
  );
}

/**
 * The job is measured from the bed's origin corner, so a work zero set on
 * the material's own corner puts it that far off the material. Shown
 * whenever the material isn't in the corner.
 */
export function zeroWarning(s: { origin: string; stockLeft: number; stockFront: number }): string | null {
  if (s.stockLeft === 0 && s.stockFront === 0) return null;
  const corner = s.origin === "backLeft" ? "back-left" : "front-left";
  return `Kerf measures the job from the bed's ${corner} corner. If you set the work zero on the material's corner instead, put the material back in the corner.`;
}

/**
 * Shown at the top of the Machine step. The reminder to frame the job
 * appears after the material is moved and goes with OK; the warning about
 * the work zero stays for as long as the material is away from the corner.
 */
export function FrameFirstNote() {
  const framePrompt = useCamera((s) => s.framePrompt);
  const origin = useProject((s) => s.origin);
  const stockLeft = useProject((s) => s.stockLeft);
  const stockFront = useProject((s) => s.stockFront);
  const zero = zeroWarning({ origin, stockLeft, stockFront });
  if (!framePrompt && !zero) return null;
  return (
    <div role="alert" className="mb-4 flex items-start gap-2 rounded-lg border border-accent bg-raised px-3 py-2">
      <TriangleAlert size={15} className="mt-0.5 shrink-0 text-accent" />
      <span className="flex flex-1 flex-col gap-1">
        {framePrompt && (
          <span className="flex items-start gap-2">
            <span className="flex-1">
              {framePrompt === "found" ? "The material was placed from the camera." : "The material has been moved."} Frame the job
              to check it sits on the material before you start.
            </span>
            <button className="text-muted hover:text-fg" onClick={useCamera.getState().dismissFramePrompt}>
              OK
            </button>
          </span>
        )}
        {zero && <span>{zero}</span>}
      </span>
    </div>
  );
}
