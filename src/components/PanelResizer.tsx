import { useRef, type KeyboardEvent, type PointerEvent } from "react";
import { PANEL_WIDTHS, useUi, type Panel } from "../state/ui";

/** The least room, in pixels, the canvas is left with however wide the panels are dragged. */
const CANVAS_MIN = 360;
/** How far an arrow key moves the edge, in pixels. */
const KEY_STEP = 16;

/**
 * The left edge of a side panel, which can be dragged to make the panel wider
 * or narrower. Double-clicking it goes back to the usual width. The panel it
 * sits in must be positioned (`relative`).
 */
export function PanelResizer({ panel, label }: { panel: Panel; label: string }) {
  const width = useUi((s) => s.panels[panel]);
  const drag = useRef<{ x: number; width: number; room: number } | null>(null);
  const { min, max } = PANEL_WIDTHS[panel];

  function onPointerDown(e: PointerEvent<HTMLDivElement>) {
    if (e.button !== 0) return;
    e.preventDefault();
    e.currentTarget.setPointerCapture(e.pointerId);
    // What the canvas has to spare is the most this panel can grow by.
    const canvas = document.querySelector("main")?.getBoundingClientRect().width ?? CANVAS_MIN;
    drag.current = { x: e.clientX, width, room: Math.max(0, canvas - CANVAS_MIN) };
  }

  function onPointerMove(e: PointerEvent<HTMLDivElement>) {
    const from = drag.current;
    if (!from) return;
    // The panel is on the right, so dragging its edge to the left widens it.
    const wanted = from.width + (from.x - e.clientX);
    useUi.getState().setPanelWidth(panel, Math.min(wanted, from.width + from.room));
  }

  function onKeyDown(e: KeyboardEvent<HTMLDivElement>) {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight" && e.key !== "Home") return;
    e.preventDefault();
    const { setPanelWidth } = useUi.getState();
    if (e.key === "Home") setPanelWidth(panel);
    else setPanelWidth(panel, width + (e.key === "ArrowLeft" ? KEY_STEP : -KEY_STEP));
  }

  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={label}
      aria-valuenow={width}
      aria-valuemin={min}
      aria-valuemax={max}
      tabIndex={0}
      title="Drag to make this panel wider or narrower. Double-click for its usual width."
      className="group absolute inset-y-0 -left-1 z-10 w-2 cursor-col-resize touch-none outline-none"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={() => (drag.current = null)}
      onPointerCancel={() => (drag.current = null)}
      onDoubleClick={() => useUi.getState().setPanelWidth(panel)}
      onKeyDown={onKeyDown}
    >
      {/* Shows where to take hold, when the pointer is over it or it has the keyboard. */}
      <div className="mx-auto h-full w-0.5 bg-accent opacity-0 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100 group-active:opacity-100" />
    </div>
  );
}