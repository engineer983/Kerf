import { useRef } from "react";
import { ACCEPTED_IMAGE_TYPES, formatBytes } from "../lib/image";
import { useProject, type Step } from "../state/project";
import { DevicePanel } from "./DevicePanel";
import { Gallery } from "./Gallery";
import { MachinePanel } from "./MachinePanel";
import { OperationsPanel } from "./OperationsPanel";
import { TracePanel } from "./TracePanel";

const TITLES: Record<Step, string> = {
  device: "Device",
  import: "Import",
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
        ) : step === "import" ? (
          <ImportPanel />
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

function ImportPanel() {
  const image = useProject((s) => s.image);
  const error = useProject((s) => s.error);
  const importFile = useProject((s) => s.importFile);
  const clearImage = useProject((s) => s.clearImage);
  const galleryError = useProject((s) => s.galleryError);
  const input = useRef<HTMLInputElement>(null);

  return (
    <div className="flex flex-col gap-4">
      <input
        ref={input}
        type="file"
        accept={ACCEPTED_IMAGE_TYPES}
        className="hidden"
        onChange={(e) => {
          const file = e.target.files?.[0];
          if (file) void importFile(file);
          e.target.value = "";
        }}
      />
      <button
        onClick={() => input.current?.click()}
        className="rounded-lg bg-accent px-3 py-2 font-medium text-accent-fg transition-opacity hover:opacity-90"
      >
        {image ? "Replace image…" : "Choose image…"}
      </button>

      {error && (
        <p role="alert" className="rounded-lg border border-accent/40 px-3 py-2 text-accent">
          {error}
        </p>
      )}

      {image ? (
        <>
          <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2">
            <dt className="text-muted">File</dt>
            <dd className="truncate select-text" title={image.name}>
              {image.name}
            </dd>
            <dt className="text-muted">Pixels</dt>
            <dd className="font-mono text-[12px]">
              {image.width} × {image.height}
            </dd>
            <dt className="text-muted">Size</dt>
            <dd className="font-mono text-[12px]">{formatBytes(image.bytes)}</dd>
          </dl>
          <button
            onClick={clearImage}
            className="self-start text-muted underline-offset-2 hover:text-fg hover:underline"
          >
            Remove image
          </button>
        </>
      ) : (
        <p className="text-muted">Or drop a file anywhere on the canvas.</p>
      )}
      {galleryError && (
        <p role="alert" className="rounded-lg border border-accent/40 px-3 py-2 text-accent">
          {galleryError}
        </p>
      )}
      <Gallery />
    </div>
  );
}
