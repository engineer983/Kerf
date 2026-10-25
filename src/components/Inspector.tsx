import { CircleHelp } from "lucide-react";
import { topicForStep } from "../lib/help";
import { useProject, type Step } from "../state/project";
import { useUi } from "../state/ui";
import { DevicePanel } from "./DevicePanel";
import { DesignStep } from "./DesignStep";
import { FrameFirstNote } from "./FoundMaterial";
import { MachinePanel } from "./MachinePanel";
import { MaterialPanel } from "./MaterialPanel";
import { OperationsPanel } from "./OperationsPanel";
import { PanelResizer } from "./PanelResizer";
import { TracePanel } from "./TracePanel";

const TITLES: Record<Step, string> = {
  device: "Device",
  material: "Material",
  design: "Design",
  trace: "Trace",
  operations: "Operations",
  machine: "Machine",
};


export function Inspector() {
  const step = useProject((s) => s.step);
  const machineKind = useProject((s) => s.machineKind);
  const width = useUi((s) => s.panels.inspector);

  return (
    <aside className="relative flex shrink-0 flex-col border-l border-line bg-surface" style={{ width }}>
      <PanelResizer panel="inspector" label="Width of the settings panel" />
      <div className="flex items-center border-b border-line px-5 py-3">
        <h2 className="text-[13px] font-semibold">{TITLES[step]}</h2>
        <button
          onClick={() => useUi.getState().setHelp(topicForStep(step, machineKind))}
          aria-label={`Help with ${TITLES[step]}`}
          title="Open the guide at the part about this step"
          className="ml-auto rounded p-0.5 text-muted transition-colors hover:text-fg"
        >
          <CircleHelp size={15} />
        </button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto p-5 [scrollbar-gutter:stable]">
        {step === "device" ? (
          <DevicePanel />
        ) : step === "material" ? (
          <MaterialPanel />
        ) : step === "design" ? (
          <DesignStep />
        ) : step === "trace" ? (
          <TracePanel />
        ) : step === "operations" ? (
          <OperationsPanel />
        ) : (
          <>
            <FrameFirstNote />
            <MachinePanel />
          </>
        )}
      </div>
    </aside>
  );
}
