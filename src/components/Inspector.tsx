import { useEffect, useRef, useState } from "react";
import { ACCEPTED_FILE_TYPES, formatBytes } from "../lib/image";
import { inkscapeFound, isTauri } from "../lib/ipc";
import { useProject, type Step } from "../state/project";
import { useUi } from "../state/ui";
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
  // With Inkscape installed, more kinds of drawing can be opened.
  const [inkscape, setInkscape] = useState(false);
  const [opening, setOpening] = useState(false);
  useEffect(() => {
    if (isTauri()) inkscapeFound().then(setInkscape, () => setInkscape(false));
  }, []);

  async function openDrawing() {
    setOpening(true);
    try {
      await useProject.getState().openDrawing();
    } catch (e) {
      useProject.setState({ error: e instanceof Error ? e.message : String(e) });
    } finally {
      setOpening(false);
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <input
        ref={input}
        type="file"
        accept={ACCEPTED_FILE_TYPES}
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
        <p className="text-muted">
          Or drop a file anywhere on the canvas; an SVG is used as it is drawn, with no tracing. A picture
          copied in your web browser (right-click, Copy image) can be pasted with Ctrl+V, and so can a
          picture's web address.
        </p>
      )}
      <div className="flex flex-col gap-2 border-t border-line pt-4">
        <h3 className="text-[11px] font-semibold tracking-wider text-muted uppercase">Or start from lines</h3>
        <button
          onClick={() => void openDrawing()}
          disabled={opening}
          title={
            inkscape
              ? "Open an SVG, PDF, AI or EPS file and use its lines exactly as drawn. Inkscape converts the last three."
              : "Open an SVG file and use its lines exactly as drawn. With Inkscape installed, PDF, AI and EPS files can be opened too."
          }
          className="rounded-lg border border-line px-3 py-2 text-left transition-colors hover:border-accent hover:text-accent disabled:opacity-40"
        >
          {opening ? "Opening…" : inkscape ? "Open a drawing… (SVG, PDF, AI, EPS)" : "Open a drawing… (SVG)"}
        </button>
        <button
          onClick={() => useUi.getState().setMaker("text")}
          title="Type words and draw them in any font on this computer"
          className="rounded-lg border border-line px-3 py-2 text-left transition-colors hover:border-accent hover:text-accent"
        >
          Text…
        </button>
        <button
          onClick={() => useUi.getState().setMaker("box")}
          title="Draw the panels of a box with finger joints, from its size and your material's thickness"
          className="rounded-lg border border-line px-3 py-2 text-left transition-colors hover:border-accent hover:text-accent"
        >
          Box maker…
        </button>
        <button
          onClick={() => useUi.getState().setMaker("layers")}
          disabled={!image}
          title="Slice the picture into layers to cut from thin sheet and stack"
          className="rounded-lg border border-line px-3 py-2 text-left transition-colors hover:border-accent hover:text-accent disabled:opacity-40"
        >
          Stacked layers…
        </button>
        {!image && <p className="text-muted">Stacked layers are made from a picture, so import one first.</p>}
      </div>
      {galleryError && (
        <p role="alert" className="rounded-lg border border-accent/40 px-3 py-2 text-accent">
          {galleryError}
        </p>
      )}
      <Gallery />
    </div>
  );
}
