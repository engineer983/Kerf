import { useRef } from "react";
import type { Rect } from "../lib/simulation";

/** Compass names: a corner has two letters, an edge one. */
type Handle = "nw" | "n" | "ne" | "e" | "se" | "s" | "sw" | "w";
type Grip = Handle | "move";

const HANDLES: { id: Handle; fx: number; fy: number; cursor: string }[] = [
  { id: "nw", fx: 0, fy: 0, cursor: "nwse-resize" },
  { id: "n", fx: 0.5, fy: 0, cursor: "ns-resize" },
  { id: "ne", fx: 1, fy: 0, cursor: "nesw-resize" },
  { id: "e", fx: 1, fy: 0.5, cursor: "ew-resize" },
  { id: "se", fx: 1, fy: 1, cursor: "nwse-resize" },
  { id: "s", fx: 0.5, fy: 1, cursor: "ns-resize" },
  { id: "sw", fx: 0, fy: 1, cursor: "nesw-resize" },
  { id: "w", fx: 0, fy: 0.5, cursor: "ew-resize" },
];

const MIN_SIZE_MM = 1;
const HANDLE_PX = 10;

/** The rectangle after dragging `grip` by (dx, dy) from where `start` was. */
export function dragRect(start: Rect, grip: Grip, dx: number, dy: number, lockAspect: boolean, bounds?: Rect): Rect {
  const { x, y, width, height } = start;
  if (grip === "move") {
    let nx = x + dx;
    let ny = y + dy;
    if (bounds) {
      nx = Math.min(bounds.x + bounds.width - width, Math.max(bounds.x, nx));
      ny = Math.min(bounds.y + bounds.height - height, Math.max(bounds.y, ny));
    }
    return { x: nx, y: ny, width, height };
  }

  let [left, top, right, bottom] = [x, y, x + width, y + height];
  if (grip.includes("w")) left = Math.min(left + dx, right - MIN_SIZE_MM);
  if (grip.includes("e")) right = Math.max(right + dx, left + MIN_SIZE_MM);
  if (grip.includes("n")) top = Math.min(top + dy, bottom - MIN_SIZE_MM);
  if (grip.includes("s")) bottom = Math.max(bottom + dy, top + MIN_SIZE_MM);

  if (lockAspect) {
    let newWidth = right - left;
    let newHeight = bottom - top;
    if (grip.length === 2) {
      // A corner follows whichever direction was dragged further, and the
      // opposite corner stays put.
      const scale = Math.max(newWidth / width, newHeight / height);
      newWidth = width * scale;
      newHeight = height * scale;
      left = grip.includes("w") ? x + width - newWidth : x;
      top = grip.includes("n") ? y + height - newHeight : y;
    } else if (grip === "e" || grip === "w") {
      // An edge scales the other direction about its middle.
      newHeight = (newWidth * height) / width;
      top = y + (height - newHeight) / 2;
    } else {
      newWidth = (newHeight * width) / height;
      left = x + (width - newWidth) / 2;
    }
    right = left + newWidth;
    bottom = top + newHeight;
  }

  if (bounds) {
    left = Math.max(bounds.x, left);
    top = Math.max(bounds.y, top);
    right = Math.min(bounds.x + bounds.width, right);
    bottom = Math.min(bounds.y + bounds.height, bottom);
  }
  return { x: left, y: top, width: right - left, height: bottom - top };
}

interface HandlesProps {
  rect: Rect;
  /** Millimetres per screen pixel, so that handles keep their size on screen. */
  px: number;
  lockAspect: boolean;
  /** The rectangle must stay inside this one. */
  bounds?: Rect;
  dashed?: boolean;
  /** The rectangle can be moved but not resized. */
  moveOnly?: boolean;
  /** Converts a pointer position to bed coordinates. */
  toBed: (event: { clientX: number; clientY: number }) => [number, number];
  onChange: (rect: Rect) => void;
}

/** A frame around a rectangle that can be dragged to move it or resize it. */
export function Handles({ rect, px, lockAspect, bounds, dashed, moveOnly, toBed, onChange }: HandlesProps) {
  const drag = useRef<{ grip: Grip; from: [number, number]; start: Rect } | null>(null);

  const grab = (grip: Grip) => ({
    onPointerDown: (e: React.PointerEvent) => {
      if (e.button !== 0) return;
      // Dragging the artwork must not also pan the canvas underneath.
      e.stopPropagation();
      e.currentTarget.setPointerCapture(e.pointerId);
      drag.current = { grip, from: toBed(e), start: rect };
    },
    onPointerMove: (e: React.PointerEvent) => {
      const d = drag.current;
      if (!d) return;
      const [bx, by] = toBed(e);
      onChange(dragRect(d.start, d.grip, bx - d.from[0], by - d.from[1], lockAspect, bounds));
    },
    onPointerUp: () => {
      drag.current = null;
    },
    onPointerCancel: () => {
      drag.current = null;
    },
    onDoubleClick: (e: React.MouseEvent) => e.stopPropagation(),
  });

  const size = HANDLE_PX * px;
  return (
    <g style={{ pointerEvents: "auto" }}>
      <rect
        x={rect.x}
        y={rect.y}
        width={rect.width}
        height={rect.height}
        fill="transparent"
        stroke="var(--accent)"
        strokeWidth={px * 1.5}
        strokeDasharray={dashed ? `${px * 6} ${px * 4}` : undefined}
        style={{ cursor: "move" }}
        {...grab("move")}
      />
      {(moveOnly ? [] : HANDLES).map((h) => (
        <rect
          key={h.id}
          x={rect.x + rect.width * h.fx - size / 2}
          y={rect.y + rect.height * h.fy - size / 2}
          width={size}
          height={size}
          rx={px * 2}
          fill="var(--raised)"
          stroke="var(--accent)"
          strokeWidth={px * 1.5}
          style={{ cursor: h.cursor }}
          {...grab(h.id)}
        />
      ))}
    </g>
  );
}
