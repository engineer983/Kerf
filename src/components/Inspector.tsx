import { useProject, type Step } from "../state/project";
import { DevicePanel } from "./DevicePanel";
import { DesignStep } from "./DesignStep";
import { MachinePanel } from "./MachinePanel";
import { MaterialPanel } from "./MaterialPanel";
import { OperationsPanel } from "./OperationsPanel";
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

  return (
    <aside className="flex w-[300px] shrink-0 flex-col border-l border-line bg-surface">
      <h2 className="border-b border-line px-5 py-3 text-[13px] font-semibold">{TITLES[step]}</h2>
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
          <MachinePanel />
        )}
      </div>
    </aside>
  );
}
