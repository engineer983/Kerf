import { useEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import type { HintContent } from "../lib/hints";
import { Demo } from "./Demos";

const CARD_WIDTH = 250;
/** How long the pointer must rest on a label before its hint appears. */
const DELAY_MS = 350;

interface HintProps {
  hint?: HintContent;
  children: ReactNode;
}

/**
 * Wraps a label so that resting the pointer on it, or focusing it, shows an
 * explanation of the setting, with an animated diagram where there is one.
 */
export function Hint({ hint, children }: HintProps) {
  const anchor = useRef<HTMLSpanElement>(null);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const [position, setPosition] = useState<{ left: number; top: number } | null>(null);

  useEffect(() => () => clearTimeout(timer.current), []);

  if (!hint) return <>{children}</>;

  const show = () => {
    clearTimeout(timer.current);
    timer.current = setTimeout(() => {
      const box = anchor.current?.getBoundingClientRect();
      if (!box) return;
      // Beside the panel rather than over it, so the control stays visible.
      const beside = box.left - CARD_WIDTH - 28;
      const left = beside >= 8 ? beside : Math.min(window.innerWidth - CARD_WIDTH - 8, box.right + 12);
      setPosition({ left, top: Math.max(8, Math.min(window.innerHeight - 220, box.top - 8)) });
    }, DELAY_MS);
  };
  const hide = () => {
    clearTimeout(timer.current);
    setPosition(null);
  };

  return (
    <span
      ref={anchor}
      className="cursor-help underline decoration-line decoration-dotted underline-offset-4"
      onMouseEnter={show}
      onMouseLeave={hide}
      onFocus={show}
      onBlur={hide}
    >
      {children}
      {position &&
        createPortal(
          <div
            role="tooltip"
            className="pointer-events-none fixed z-[60] flex flex-col gap-2 rounded-xl border border-line bg-surface p-3 text-[12px] leading-relaxed text-fg shadow-xl"
            style={{ left: position.left, top: position.top, width: CARD_WIDTH }}
          >
            {hint.demo && <Demo id={hint.demo} />}
            {hint.text}
          </div>,
          document.body,
        )}
    </span>
  );
}
