import { ImagePlus, Layers, Settings2, Spline, Zap } from "lucide-react";
import { MACHINE_PRESETS } from "../lib/machines";
import { STEP_ORDER, useProject, type Step } from "../state/project";

const STEPS: Record<Step, { label: string; icon: typeof Zap }> = {
  device: { label: "Device", icon: Settings2 },
  import: { label: "Import", icon: ImagePlus },
  trace: { label: "Trace", icon: Spline },
  operations: { label: "Operations", icon: Layers },
  machine: { label: "Machine", icon: Zap },
};

/** The steps of a job, numbered in the order they are worked through. */
export function Rail() {
  const step = useProject((s) => s.step);
  const setStep = useProject((s) => s.setStep);
  // A machine that only takes G-code files has nothing to connect to.
  const exportOnly = useProject(
    (s) => MACHINE_PRESETS.find((p) => p.id === s.machinePreset)?.exportOnly ?? false,
  );

  return (
    <nav
      aria-label="Workflow"
      className="flex w-[76px] shrink-0 flex-col gap-1 border-r border-line bg-surface px-2 py-3"
    >
      {STEP_ORDER.filter((id) => !(id === "machine" && exportOnly)).map((id, index) => {
        const { label, icon: Icon } = STEPS[id];
        const active = step === id;
        return (
          <button
            key={id}
            onClick={() => setStep(id)}
            aria-label={label}
            aria-current={active ? "step" : undefined}
            className={`flex flex-col items-center gap-1 rounded-lg py-2.5 text-[11px] transition-colors ${
              active ? "bg-raised text-accent shadow-sm" : "text-muted hover:text-fg"
            }`}
          >
            <span className="flex items-center gap-1.5">
              <span
                className={`flex h-[18px] w-[18px] items-center justify-center rounded-full font-mono text-[11px] font-semibold ${
                  active ? "bg-accent text-accent-fg" : "border border-line"
                }`}
              >
                {index + 1}
              </span>
              <Icon size={18} strokeWidth={1.75} />
            </span>
            {label}
          </button>
        );
      })}
    </nav>
  );
}
