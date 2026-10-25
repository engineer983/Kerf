/**
 * Jog speeds and the remembered jog choices for the Machine panel.
 */

export type JogSpeed = "slow" | "normal" | "fast";
/** A step in millimetres, or "hold" to move only while a button or key is held. */
export type JogStep = "0.1" | "1" | "10" | "100" | "hold";

export const JOG_STEPS: { value: JogStep; label: string }[] = [
  { value: "0.1", label: "0.1" },
  { value: "1", label: "1" },
  { value: "10", label: "10" },
  { value: "100", label: "100" },
  { value: "hold", label: "Hold" },
];

export const JOG_SPEEDS: { value: JogSpeed; label: string }[] = [
  { value: "slow", label: "Slow" },
  { value: "normal", label: "Normal" },
  { value: "fast", label: "Fast" },
];

/** X and Y speeds in mm/min; Z moves at one fifth of these. */
const XY_FEED: Record<JogSpeed, number> = { slow: 300, normal: 1000, fast: 3000 };

const SPEED_KEY = "kerf.jogSpeed";
const STEP_KEY = "kerf.jogStep";

/**
 * The speed for moving `axis` (0 = X, 1 = Y, 2 = Z) in mm/min, capped at the
 * machine's own maximum rate for that axis ($110 to $112) when it is known.
 */
export function jogFeed(speed: JogSpeed, axis: number, settings: Record<number, string>): number {
  const feed = axis === 2 ? XY_FEED[speed] / 5 : XY_FEED[speed];
  const max = Number(settings[110 + axis]);
  return Number.isFinite(max) && max > 0 ? Math.min(feed, max) : feed;
}

function stored<T extends string>(key: string, options: { value: T }[], fallback: T): T {
  try {
    const value = localStorage.getItem(key);
    return options.find((o) => o.value === value)?.value ?? fallback;
  } catch {
    return fallback;
  }
}

function store(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Remembering the choice is a convenience; without storage it resets.
  }
}

export const storedJogSpeed = () => stored(SPEED_KEY, JOG_SPEEDS, "normal");
export const storedJogStep = () => stored(STEP_KEY, JOG_STEPS, "1");
export const storeJogSpeed = (speed: JogSpeed) => store(SPEED_KEY, speed);
export const storeJogStep = (step: JogStep) => store(STEP_KEY, step);
