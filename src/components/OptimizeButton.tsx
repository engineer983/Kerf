import { useState } from "react";
import { Wand2 } from "lucide-react";
import { useProject } from "../state/project";

/** Tunes the picture's adjustments to what is being made from it. */
export function OptimizeButton() {
  const note = useProject((s) => s.optimizeNote);
  const hasImage = useProject((s) => s.image !== null);
  const [working, setWorking] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);

  async function run() {
    setWorking(true);
    setProblem(null);
    try {
      await useProject.getState().optimizePicture();
    } catch (e) {
      setProblem(e instanceof Error ? e.message : String(e));
    } finally {
      setWorking(false);
    }
  }

  return (
    <div className="flex flex-col gap-2">
      <button
        onClick={() => void run()}
        disabled={!hasImage || working}
        title="Set the brightness, contrast, smoothing and trace settings to suit what you are making. Undo with Ctrl+Z."
        className="flex items-center justify-center gap-2 rounded-lg border border-line px-3 py-2 transition-colors hover:border-accent hover:text-accent disabled:opacity-40"
      >
        <Wand2 size={14} />
        {working ? "Optimising…" : "Optimise picture"}
      </button>
      {(problem ?? note) && (
        <p role={problem ? "alert" : "status"} className={problem ? "text-accent" : "text-muted"}>
          {problem ?? note}
        </p>
      )}
    </div>
  );
}
