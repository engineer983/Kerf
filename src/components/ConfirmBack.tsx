import { useEffect, useState } from "react";
import { useProject } from "../state/project";

const NAMES = { device: "Device", material: "Material", design: "Design", trace: "Trace", operations: "Operations", machine: "Machine" };

/** Asks before going back to the picture throws away the job that was generated. */
export function ConfirmBack() {
  const pending = useProject((s) => s.pendingStep);
  const { confirmStep, cancelStep } = useProject.getState();
  const [dontWarn, setDontWarn] = useState(false);

  useEffect(() => {
    if (!pending) return;
    setDontWarn(false);
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && cancelStep();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [pending, cancelStep]);

  if (!pending) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-8" onClick={cancelStep}>
      <div
        role="alertdialog"
        aria-label="Go back?"
        className="flex w-full max-w-[420px] flex-col gap-4 rounded-2xl border border-line bg-surface p-6 shadow-xl"
        onClick={(e) => e.stopPropagation()}
      >
        <h2 className="text-[15px] font-semibold">Go back to {NAMES[pending]}?</h2>
        <p className="leading-relaxed">
          The job you generated will be cleared: its tool moves, time estimate and preview. Your picture
          and all your settings are kept, and you can generate the job again when you come back.
        </p>
        <label className="flex items-center gap-2 text-muted">
          <input type="checkbox" className="accent-accent" checked={dontWarn} onChange={(e) => setDontWarn(e.target.checked)} />
          Don't warn me about this again
        </label>
        <div className="flex justify-end gap-2">
          <button
            autoFocus
            onClick={cancelStep}
            title="Keep the generated job and stay on this step"
            className="rounded-lg border border-line px-4 py-2 transition-colors hover:text-fg"
          >
            Stay here
          </button>
          <button
            onClick={() => confirmStep(dontWarn)}
            title="Clear the generated job and go back"
            className="rounded-lg bg-accent px-4 py-2 font-medium text-accent-fg hover:opacity-90"
          >
            Go back
          </button>
        </div>
      </div>
    </div>
  );
}
