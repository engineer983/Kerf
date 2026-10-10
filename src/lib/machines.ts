import type { MachineKind } from "./ipc";

export interface MachinePreset {
  id: string;
  label: string;
  kind: MachineKind;
  /** Work area in millimetres. */
  bed: { width: number; height: number };
  maxPowerS: number;
  /** Shown under the preset; say what is unverified or unusual. */
  note?: string;
}

export const CUSTOM_PRESET = "custom";

// Figures come from published specifications and reviews, not from the machines'
// own `$$` settings. Compare with `$130`/`$131`/`$30` on the real machine.
export const MACHINE_PRESETS: MachinePreset[] = [
  {
    id: "x-carve-1000",
    label: "Inventables X-Carve 1000 mm",
    kind: "cnc",
    bed: { width: 750, height: 750 },
    maxPowerS: 1000,
    note: "Work area assumed to be 750 × 750 mm; check $130 and $131 on your machine.",
  },
  {
    id: "wecreat-vision-pro",
    label: "WeCreat Vision Pro 45W",
    kind: "laser",
    bed: { width: 500, height: 320 },
    maxPowerS: 1000,
    note: "This machine's firmware is not standard GRBL and the full-power S value is unconfirmed. Treat the G-code as untested: run at low power first and keep the lid closed.",
  },
];
