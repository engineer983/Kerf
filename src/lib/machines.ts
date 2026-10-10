import type { MachineKind, Origin } from "./ipc";

export interface MachinePreset {
  id: string;
  label: string;
  kind: MachineKind;
  /** Work area in millimetres. */
  bed: { width: number; height: number };
  maxPowerS: number;
  origin: Origin;
  /** Vendor commands the machine needs around every job. */
  startGcode: string;
  endGcode: string;
  /** Shown under the preset; say what is unverified or unusual. */
  note?: string;
}

export const CUSTOM_PRESET = "custom";

export const MACHINE_PRESETS: MachinePreset[] = [
  {
    // From published specifications, not from a machine's own `$$` settings.
    id: "x-carve-1000",
    label: "Inventables X-Carve 1000 mm",
    kind: "cnc",
    bed: { width: 750, height: 750 },
    maxPowerS: 1000,
    origin: "frontLeft",
    startGcode: "",
    endGcode: "",
    note: "Work area assumed to be 750 × 750 mm; check $130 and $131 on your machine.",
  },
  {
    // Bed size, S scale and start/end commands match WeCreat's LightBurn device
    // profile for firmware 1.0.7. The firmware is not GRBL: it acknowledges
    // lines with `ok` but has no `$$` settings and no `?` status reports.
    id: "wecreat-vision-pro",
    label: "WeCreat Vision Pro 45W",
    kind: "laser",
    bed: { width: 500, height: 320 },
    maxPowerS: 1000,
    origin: "backLeft",
    startGcode: "M14S1\nM15S1\nM16",
    endGcode: "M6",
    note: "Untested on the machine. Kerf does not set the focus height, so focus the laser first. The Y direction is inferred: check it with a small, low-power, asymmetric test and keep the lid closed.",
  },
];
