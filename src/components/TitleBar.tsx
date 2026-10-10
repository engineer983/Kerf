import { useEffect, useState } from "react";
import { MessageSquare, Monitor, Moon, Sun, Wrench } from "lucide-react";
import { getAppInfo } from "../lib/ipc";
import { MACHINE_PRESETS } from "../lib/machines";
import { jobIsActive, useMachine } from "../state/machine";
import { useProject } from "../state/project";
import { useToolbox } from "../state/tools";
import { useUi, type ThemeChoice } from "../state/ui";

const THEMES: { value: ThemeChoice; label: string; icon: typeof Sun }[] = [
  { value: "light", label: "Light theme", icon: Sun },
  { value: "system", label: "Match system theme", icon: Monitor },
  { value: "dark", label: "Dark theme", icon: Moon },
];

export function TitleBar() {
  const imageName = useProject((s) => s.image?.name);
  const { theme, setTheme, chatOpen, toggleChat } = useUi();
  const jobRunning = useMachine((s) => jobIsActive(s.job));
  const isRouter = useProject((s) => s.machineKind === "cnc");
  const deviceName = useProject(
    (s) =>
      MACHINE_PRESETS.find((p) => p.id === s.machinePreset)?.label ??
      (s.machineKind === "cnc" ? "CNC router" : "Laser"),
  );
  const [version, setVersion] = useState<string | null>(null);

  useEffect(() => {
    getAppInfo()
      .then((info) => setVersion(info?.version ?? null))
      .catch(() => setVersion(null));
  }, []);

  return (
    <header className="flex h-11 shrink-0 items-center gap-3 border-b border-line bg-surface px-4">
      <span className="text-[13px] font-semibold tracking-tight">Kerf</span>
      {version && <span className="font-mono text-[11px] text-muted">{version}</span>}
      <span className="mx-1 h-4 w-px bg-line" />
      <span className="truncate text-muted">{imageName ?? "Untitled project"}</span>

      <div className="ml-auto flex items-center gap-2">
        {jobRunning && (
          // Always in view while the machine is cutting, whichever step is open.
          <button
            onClick={() => void useMachine.getState().act({ type: "stopJob" })}
            className="rounded-md bg-accent px-3 py-1 font-semibold text-accent-fg"
          >
            Stop machine
          </button>
        )}
        <button
          onClick={() => useProject.getState().setStep("device")}
          title="Change the machine"
          className="rounded-md border border-line px-2.5 py-1 text-muted transition-colors hover:text-fg"
        >
          {deviceName}
        </button>
        {isRouter && (
          <button
            onClick={() => useToolbox.getState().setOpen(true)}
            className="flex items-center gap-1.5 rounded-md border border-line px-2.5 py-1 text-muted transition-colors hover:text-fg"
          >
            <Wrench size={14} />
            Toolbox
          </button>
        )}
        <div className="flex rounded-md border border-line p-0.5" role="radiogroup" aria-label="Theme">
          {THEMES.map(({ value, label, icon: Icon }) => (
            <button
              key={value}
              role="radio"
              aria-checked={theme === value}
              aria-label={label}
              title={label}
              onClick={() => setTheme(value)}
              className={`rounded p-1 transition-colors ${
                theme === value ? "bg-raised text-fg shadow-sm" : "text-muted hover:text-fg"
              }`}
            >
              <Icon size={14} />
            </button>
          ))}
        </div>
        <button
          onClick={toggleChat}
          aria-pressed={chatOpen}
          className={`flex items-center gap-1.5 rounded-md border px-2.5 py-1 transition-colors ${
            chatOpen
              ? "border-accent text-accent"
              : "border-line text-muted hover:text-fg"
          }`}
        >
          <MessageSquare size={14} />
          Assistant
        </button>
      </div>
    </header>
  );
}
