import { useEffect, useId, useState, type ReactNode } from "react";
import { hintFor, type HintContent } from "../lib/hints";
import { Hint } from "./Hint";

/** Every labelled control explains itself: by a hint given to it, or one found by its label. */
interface Hinted {
  hint?: HintContent;
}

export function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-3 border-b border-line pb-5 last:border-b-0">
      <h3 className="text-[11px] font-semibold uppercase tracking-wider text-muted">{title}</h3>
      {children}
    </section>
  );
}

interface SliderProps extends Hinted {
  label: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  disabled?: boolean;
  /** Shown after the number, such as "px". */
  unit?: string;
  /** The value is a fraction, shown and typed as a percentage. */
  percent?: boolean;
  onChange: (value: number) => void;
}

const decimalsOf = (step: number) => (String(step).split(".")[1] ?? "").length;
/** Whether a change came from the field's arrows rather than from typing. */
const fromArrows = (event: { nativeEvent: Event }) => !(event.nativeEvent as InputEvent).inputType;
const numberBox =
  "rounded-md border border-line bg-raised py-0.5 pr-0.5 pl-1.5 text-right font-mono text-[12px] select-text disabled:opacity-60";

/**
 * A slider whose value can also be typed, or nudged with the arrows beside the
 * number (or the up and down keys) for finer steps than dragging allows.
 */
export function Slider({ label, value, min, max, step = 1, disabled, unit, percent, hint, onChange }: SliderProps) {
  const id = useId();
  const scale = percent ? 100 : 1;
  // Percentages are nudged a point at a time, however coarse the slider.
  const shownStep = percent ? 1 : step;
  const decimals = decimalsOf(shownStep);
  const shown = (v: number) => String(Number((v * scale).toFixed(decimals)));
  const [text, setText] = useState(shown(value));
  // The box follows the value, except while what is typed already means it.
  useEffect(() => {
    setText((typed) => (typed.trim() !== "" && Number(typed) === Number(shown(value)) ? typed : shown(value)));
  }, [value]);

  /** Uses what is in the box, if it is a number; out-of-range numbers wait until typing ends. */
  function apply(typed: string, finished: boolean) {
    const parsed = Number(typed);
    if (typed.trim() === "" || !Number.isFinite(parsed)) {
      if (finished) setText(shown(value));
      return;
    }
    const wanted = parsed / scale;
    const within = Math.min(max, Math.max(min, wanted));
    if (within !== wanted && !finished) return;
    const next = Number(within.toFixed(6));
    if (next !== value) onChange(next);
    if (finished) setText(shown(next));
  }

  return (
    <div className={disabled ? "opacity-40" : undefined}>
      <div className="mb-1 flex items-center justify-between gap-2">
        <label htmlFor={id}>
          <Hint hint={hint ?? hintFor(label)}>{label}</Hint>
        </label>
        <span className="flex items-center gap-1 text-muted">
          <input
            type="number"
            aria-label={`${label} value`}
            title="Type a value, or use the arrows for small steps"
            className={`${numberBox} w-[4.5rem]`}
            step={shownStep}
            value={text}
            disabled={disabled}
            onChange={(e) => {
              setText(e.target.value);
              apply(e.target.value, fromArrows(e));
            }}
            onBlur={() => apply(text, true)}
            onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
          />
          {(unit || percent) && <span className="font-mono text-[12px]">{percent ? "%" : unit}</span>}
        </span>
      </div>
      <input
        id={id}
        type="range"
        className="w-full accent-accent"
        min={min}
        max={max}
        step={step}
        value={value}
        disabled={disabled}
        onChange={(e) => onChange(Number(e.target.value))}
      />
    </div>
  );
}

interface CheckboxProps extends Hinted {
  label: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
}

export function Checkbox({ label, checked, hint, onChange }: CheckboxProps) {
  return (
    <label className="flex items-center gap-2">
      <input
        type="checkbox"
        className="accent-accent"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
      />
      <Hint hint={hint ?? hintFor(label)}>{label}</Hint>
    </label>
  );
}

interface SegmentedProps<T extends string> {
  label: string;
  value: T;
  options: { value: T; label: string }[];
  onChange: (value: T) => void;
}

export function Segmented<T extends string>({ label, value, options, onChange }: SegmentedProps<T>) {
  return (
    <div role="radiogroup" aria-label={label} className="flex rounded-lg border border-line p-0.5">
      {options.map((option) => (
        <button
          key={option.value}
          role="radio"
          aria-checked={value === option.value}
          onClick={() => onChange(option.value)}
          className={`flex-1 rounded-md py-1.5 transition-colors ${
            value === option.value ? "bg-raised font-medium shadow-sm" : "text-muted hover:text-fg"
          }`}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

interface SelectProps<T extends string> extends Hinted {
  label: string;
  value: T;
  options: { value: T; label: string }[];
  onChange: (value: T) => void;
}

export function Select<T extends string>({ label, value, options, hint, onChange }: SelectProps<T>) {
  const id = useId();
  return (
    <div className="flex items-center justify-between gap-3">
      <label htmlFor={id} className="whitespace-nowrap">
        <Hint hint={hint ?? hintFor(label)}>{label}</Hint>
      </label>
      <select
        id={id}
        className="max-w-44 min-w-0 flex-1 rounded-md border border-line bg-raised px-2 py-1"
        value={value}
        onChange={(e) => onChange(e.target.value as T)}
      >
        {options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </select>
    </div>
  );
}

interface NumberFieldProps extends Hinted {
  label: string;
  value: number;
  min: number;
  max: number;
  unit?: string;
  /** How far one press of an arrow moves the value. Chosen to suit the unit if not given. */
  step?: number;
  onChange: (value: number) => void;
}

const STEP_FOR_UNIT: Record<string, number> = { mm: 0.1, "mm/min": 10, rpm: 100, "%": 1, "°": 0.5 };

/**
 * Typed numbers commit on blur or Enter, so half-typed ones never reach the
 * project. The arrows beside the number, and the up and down keys, nudge it
 * and take effect at once.
 */
export function NumberField({ label, value, min, max, unit, step, hint, onChange }: NumberFieldProps) {
  const id = useId();
  const [text, setText] = useState(String(value));
  useEffect(() => setText(String(value)), [value]);

  function commit(typed: string) {
    const parsed = Number(typed);
    if (typed.trim() !== "" && Number.isFinite(parsed)) {
      const clamped = Number(Math.min(max, Math.max(min, parsed)).toFixed(6));
      if (clamped !== value) onChange(clamped);
      setText(String(clamped));
    } else {
      setText(String(value));
    }
  }

  return (
    <div className="flex items-center justify-between gap-3">
      <label htmlFor={id}>
        <Hint hint={hint ?? hintFor(label)}>{label}</Hint>
      </label>
      <div className="flex items-center gap-1.5">
        <input
          id={id}
          type="number"
          className={`${numberBox} w-[5.5rem] py-1`}
          step={step ?? STEP_FOR_UNIT[unit ?? ""] ?? 1}
          value={text}
          onChange={(e) => {
            setText(e.target.value);
            if (fromArrows(e)) commit(e.target.value);
          }}
          onBlur={() => commit(text)}
          onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
        />
        {unit && <span className="text-muted">{unit}</span>}
      </div>
    </div>
  );
}