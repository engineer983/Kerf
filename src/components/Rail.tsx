import { ImagePlus, Layers, Spline, Zap } from "lucide-react";
import { useProject, type Step } from "../state/project";

const STEPS: { id: Step; label: string; icon: typeof Zap }[] = [
  { id: "import", label: "Import", icon: ImagePlus },
  { id: "trace", label: "Trace", icon: Spline },
  { id: "operations", label: "Operations", icon: Layers },
  { id: "machine", label: "Machine", icon: Zap },
];

export function Rail() {
  const step = useProject((s) => s.step);
  const setStep = useProject((s) => s.setStep);

  return (
    <nav
      aria-label="Workflow"
      className="flex w-[76px] shrink-0 flex-col gap-1 border-r border-line bg-surface px-2 py-3"
    >
      {STEPS.map(({ id, label, icon: Icon }) => {
        const active = step === id;
        return (
          <button
            key={id}
            onClick={() => setStep(id)}
            aria-current={active ? "step" : undefined}
            className={`flex flex-col items-center gap-1 rounded-lg py-2.5 text-[11px] transition-colors ${
              active ? "bg-raised text-accent shadow-sm" : "text-muted hover:text-fg"
            }`}
          >
            <Icon size={18} strokeWidth={1.75} />
            {label}
          </button>
        );
      })}
    </nav>
  );
}
