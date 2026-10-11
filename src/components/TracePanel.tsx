import { isTauri } from "../lib/ipc";
import { jobUses, useProject } from "../state/project";
import { Checkbox, NumberField, Section, Segmented, Slider } from "./controls";
import { OptimizeButton } from "./OptimizeButton";

export function TracePanel() {
  const image = useProject((s) => s.image);
  const design = useProject((s) => s.design);
  const operations = useProject((s) => s.operations);
  const uses = jobUses(operations);
  const bed = useProject((s) => s.bed);
  const widthMm = useProject((s) => s.widthMm);
  const heightMm = useProject((s) => s.heightMm);
  const showSource = useProject((s) => s.showSource);
  const exportedTo = useProject((s) => s.exportedTo);
  const { settings, result, busy, refining, error } = useProject((s) => s.trace);
  const { updateTrace, updatePreprocess, setWidthMm, setShowSource, exportSvg } =
    useProject.getState();

  if (!image) return <p className="text-muted">Put a picture on the material first, in step 3, Design.</p>;
  if (design) {
    return (
      <div className="flex flex-col gap-4">
        <p className="text-muted">
          This {{ box: "box", layers: "set of layers", text: "text", drawing: "drawing" }[design.kind]} is
          already exact lines, so there is nothing to trace. You can save the lines as an SVG to use
          elsewhere.
        </p>
        <button
          onClick={() => void exportSvg()}
          className="rounded-lg bg-accent px-3 py-2 font-medium text-accent-fg transition-opacity hover:opacity-90"
        >
          Export SVG…
        </button>
        {error && (
          <p role="alert" className="rounded-lg border border-accent/40 px-3 py-2 text-accent">
            {error}
          </p>
        )}
        {exportedTo && <p className="break-all text-muted select-text">Saved to {exportedTo}</p>}
      </div>
    );
  }
  if (!isTauri()) {
    return <p className="text-muted">Tracing needs the desktop app; it is not available in a browser.</p>;
  }

  const pre = settings.preprocess;
  const manualLevel = pre.threshold.mode === "manual" ? pre.threshold.level : 128;


  return (
    <div className="flex flex-col gap-5">
      <OptimizeButton />
      {uses.picture && !uses.traced && (
        <p className="rounded-lg border border-line px-3 py-2 text-muted">
          This job works from the picture itself, shown on the canvas in the greys the machine will read.
          Brightness, Contrast, Blur and Sharpen change it. The other settings here only matter for operations
          that follow traced shapes.
        </p>
      )}
      <Segmented
        label="Trace mode"
        value={settings.mode}
        options={[
          { value: "outline", label: "Outline" },
          { value: "color", label: "Colour" },
        ]}
        onChange={(mode) => updateTrace({ mode })}
      />

      {settings.mode === "outline" ? (
        <Section title="Black and white">
          <Checkbox
            label="Automatic threshold"
            checked={pre.threshold.mode === "otsu"}
            onChange={(auto) =>
              updatePreprocess({
                threshold: auto ? { mode: "otsu" } : { mode: "manual", level: manualLevel },
              })
            }
          />
          <Slider
            label="Threshold"
            min={0}
            max={255}
            value={manualLevel}
            disabled={pre.threshold.mode === "otsu"}
            onChange={(level) => updatePreprocess({ threshold: { mode: "manual", level } })}
          />
          <Slider
            label="Brightness"
            min={-1}
            max={1}
            step={0.01}
            value={pre.brightness}
            percent
            onChange={(brightness) => updatePreprocess({ brightness })}
          />
          <Slider
            label="Contrast"
            min={-1}
            max={1}
            step={0.01}
            value={pre.contrast}
            percent
            onChange={(contrast) => updatePreprocess({ contrast })}
          />
          <Slider
            label="Blur"
            min={0}
            max={5}
            step={0.1}
            value={pre.blurSigma}
            unit="px"
            onChange={(blurSigma) => updatePreprocess({ blurSigma })}
          />
          <Slider
            label="Sharpen"
            min={0}
            max={2}
            step={0.05}
            value={pre.sharpen ?? 0}
            percent
            onChange={(sharpen) => updatePreprocess({ sharpen })}
          />
          <Checkbox
            label="Invert (trace light areas)"
            checked={pre.invert}
            onChange={(invert) => updatePreprocess({ invert })}
          />
        </Section>
      ) : (
        <Section title="Colour">
          <Slider
            label="Colour detail"
            min={1}
            max={8}
            value={settings.colorPrecision}
            onChange={(colorPrecision) => updateTrace({ colorPrecision })}
          />
          <Slider
            label="Layer separation"
            min={0}
            max={64}
            value={settings.layerDifference}
            onChange={(layerDifference) => updateTrace({ layerDifference })}
          />
        </Section>
      )}

      <Section title="Paths">
        <Slider
          label="Ignore specks under"
          min={0}
          max={16}
          value={settings.speckle}
          unit="px"
          onChange={(speckle) => updateTrace({ speckle })}
        />
        <Checkbox
          label="Smooth curves"
          checked={settings.curves}
          onChange={(curves) => updateTrace({ curves })}
        />
        <Slider
          label="Corner sharpness"
          min={0}
          max={180}
          value={settings.cornerThreshold}
          disabled={!settings.curves}
          unit="°"
          onChange={(cornerThreshold) => updateTrace({ cornerThreshold })}
        />
      </Section>

      <Section title="Output">
        <NumberField
          label="Width"
          unit="mm"
          min={1}
          max={bed.width * 10}
          value={widthMm}
          onChange={setWidthMm}
        />
        <div className="flex justify-between">
          <span>Height</span>
          <span className="font-mono text-[12px] text-muted">{heightMm.toFixed(1)} mm</span>
        </div>
        <Checkbox label="Show original image" checked={showSource} onChange={setShowSource} />
      </Section>

      <div className="flex flex-col gap-3">
        <p className="text-muted" aria-live="polite">
          {busy && !result
            ? "Tracing…"
            : result
              ? `${result.pathCount} ${result.pathCount === 1 ? "path" : "paths"}${refining ? " · refining…" : ""}`
              : ""}
        </p>
        {error && (
          <p role="alert" className="rounded-lg border border-accent/40 px-3 py-2 text-accent">
            {error}
          </p>
        )}
        <button
          onClick={() => void exportSvg()}
          disabled={!result || result.pathCount === 0}
          className="rounded-lg bg-accent px-3 py-2 font-medium text-accent-fg transition-opacity hover:opacity-90 disabled:opacity-40"
        >
          Export SVG…
        </button>
        {exportedTo && (
          <p className="break-all text-muted select-text">Saved to {exportedTo}</p>
        )}
      </div>
    </div>
  );
}
