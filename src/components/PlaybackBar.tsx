import { Pause, Play, SkipBack, SkipForward } from "lucide-react";
import { SPEEDS, usePlayback, type ViewMode } from "../state/playback";

const MODES: { value: ViewMode; label: string }[] = [
  { value: "paths", label: "Paths" },
  { value: "result", label: "Result" },
  { value: "3d", label: "3D" },
];

const icon = "rounded-md p-1.5 text-muted transition-colors hover:text-fg";

/** View switch and transport controls for replaying a generated job. */
export function PlaybackBar() {
  const { mode, playing, progress, speed, setMode, setProgress, setSpeed, toggle, finish } =
    usePlayback();

  return (
    <div
      className="absolute bottom-3 left-1/2 flex -translate-x-1/2 cursor-default items-center gap-2 rounded-xl border border-line bg-surface/95 px-2 py-1.5 shadow-sm backdrop-blur"
      // Keep clicks and drags here from panning or zooming the canvas underneath.
      onPointerDown={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
      onWheel={(e) => e.stopPropagation()}
    >
      <div className="flex rounded-lg border border-line p-0.5" role="radiogroup" aria-label="View">
        {MODES.map((m) => (
          <button
            key={m.value}
            role="radio"
            aria-checked={mode === m.value}
            onClick={() => setMode(m.value)}
            className={`rounded-md px-2.5 py-1 transition-colors ${
              mode === m.value ? "bg-raised font-medium shadow-sm" : "text-muted hover:text-fg"
            }`}
          >
            {m.label}
          </button>
        ))}
      </div>

      <button className={icon} aria-label="Back to the start" onClick={() => setProgress(0)}>
        <SkipBack size={15} />
      </button>
      <button
        className="rounded-lg bg-accent p-2 text-accent-fg transition-opacity hover:opacity-90"
        aria-label={playing ? "Pause" : "Play"}
        onClick={toggle}
      >
        {playing ? <Pause size={15} /> : <Play size={15} />}
      </button>
      <button className={icon} aria-label="Skip to the finished job" onClick={finish}>
        <SkipForward size={15} />
      </button>

      <input
        type="range"
        aria-label="Position in the job"
        className="w-56 accent-accent"
        min={0}
        max={1}
        step={0.001}
        value={progress}
        onChange={(e) => setProgress(Number(e.target.value))}
      />
      <span className="w-9 text-right font-mono text-[11px] text-muted">
        {Math.round(progress * 100)}%
      </span>

      <select
        aria-label="Playback speed"
        className="rounded-md border border-line bg-raised px-1.5 py-1 font-mono text-[11px]"
        value={speed}
        onChange={(e) => setSpeed(Number(e.target.value))}
      >
        {SPEEDS.map((s) => (
          <option key={s} value={s}>
            {s}×
          </option>
        ))}
      </select>
    </div>
  );
}
