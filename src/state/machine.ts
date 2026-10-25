import { create } from "zustand";
import * as ipc from "../lib/ipc";
import type { Firmware, JobState, MachineAction, MachineState, PartsProgress, PortInfo, ProgramPart } from "../lib/ipc";

const CONSOLE_LIMIT = 200;

interface JobProgress {
  state: JobState;
  acknowledged: number;
  total: number;
  message: string | null;
}

interface MachineStore {
  ports: PortInfo[];
  port: string | null;
  connecting: boolean;
  connected: boolean;
  /** Firmware version from the start-up banner, once seen. */
  version: string | null;
  /** Grbl or grblHAL, from the banner or `$I`. */
  firmware: Firmware | null;
  /** Input pins on in the last status report; "P" is the probe touching. */
  pins: string;
  /**
   * The machine has homed since connecting, as far as the interface has seen.
   * Only used for messages; the sender checks for itself.
   */
  homed: boolean;
  /** Where a job with bit changes has got to, from the sender. */
  parts: PartsProgress | null;
  /**
   * The running job's parts as generated when it was started, so the bit
   * named at a change is the one in the file being run.
   */
  jobParts: ProgramPart[];
  /** The running job changes bits with an automatic tool changer. */
  jobAtc: boolean;
  /** The last touch plate probe, while running or once it has ended. */
  probe: { state: "running" | "done" | "failed"; message: string | null } | null;
  /** The probe test has passed since connecting. */
  probeTested: boolean;
  /** Notes what job is about to start, then starts it. */
  startJob: (programId: number, parts: ProgramPart[], atc: boolean) => Promise<void>;
  state: MachineState | null;
  /** Work position in millimetres. */
  position: [number, number, number] | null;
  /** Position from the machine's home in millimetres, when reported. */
  machinePosition: [number, number, number] | null;
  /** Laser or spindle speed in the last status report, if it gave one. */
  spindle: number | null;
  /** GRBL `$` settings reported by the machine, by number. */
  settings: Record<number, string>;
  job: JobProgress | null;
  console: string[];
  error: string | null;
  /**
   * The XY work zero was moved this session (by "Zero XY here" or a G10/G92
   * typed in the console). The camera picture assumes zero is the bed's
   * corner, so it says so. Kept after disconnecting: the machine remembers.
   */
  originMoved: boolean;
  refreshPorts: () => Promise<void>;
  setPort: (port: string) => void;
  connect: () => Promise<void>;
  disconnect: () => Promise<void>;
  act: (action: MachineAction) => Promise<void>;
}

function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

const DISCONNECTED = {
  connecting: false,
  connected: false,
  version: null,
  firmware: null,
  pins: "",
  homed: false,
  parts: null,
  probe: null,
  probeTested: false,
  state: null,
  position: null,
  machinePosition: null,
  spindle: null,
  settings: {},
  job: null,
};

export const useMachine = create<MachineStore>((set, get) => {
  const log = (line: string) =>
    set((s) => ({ console: [...s.console, line].slice(-CONSOLE_LIMIT) }));

  if (ipc.isTauri()) {
    void ipc.onMachineEvent((event) => {
      switch (event.type) {
        case "status": {
          const previous = get().state;
          set({
            state: event.state,
            position: event.position,
            machinePosition: event.machinePosition,
            spindle: event.spindle,
            pins: event.pins,
            ...(event.state === "idle" && previous === "home" ? { homed: true } : {}),
          });
          break;
        }
        case "welcome":
          set({ version: event.version, firmware: event.firmware, settings: {}, homed: false });
          log(`Connected to ${event.firmware === "grblHal" ? "grblHAL" : "Grbl"} ${event.version}`);
          break;
        case "firmware":
          set({ firmware: event.firmware });
          break;
        case "parts":
          set({ parts: event.progress });
          break;
        case "probe":
          set({ probe: { state: event.state, message: event.message } });
          if (event.state === "failed" && event.message) log(event.message);
          break;
        case "probeTested":
          set({ probeTested: true });
          log("Probe test passed: the plate touching the bit was seen.");
          break;
        case "setting":
          set((s) => ({ settings: { ...s.settings, [event.number]: event.value } }));
          break;
        case "line":
          log(event.text);
          break;
        case "commandError":
          log(`${event.line} → error ${event.code}: ${event.description}`);
          // A refused move, such as one past the edge with soft limits on,
          // is said plainly; it also ends a held jog.
          if (event.line.startsWith("$J=")) {
            set({ error: `The machine refused the move (error ${event.code}): ${event.description}` });
          }
          break;
        case "alarm":
          log(`ALARM ${event.code}: ${event.description}`);
          set({ error: `Alarm ${event.code}: ${event.description}`, homed: false });
          break;
        case "job":
          set({ job: event });
          if (event.state === "waitingForBit") {
            log("Part finished. Waiting for the bit to be changed.");
          } else if (event.state !== "running") {
            log(event.message ?? `Job ${event.state} (${event.acknowledged} of ${event.total} lines).`);
          }
          break;
        case "disconnected":
          set({
            ...DISCONNECTED,
            // Once the port is gone Kerf can no longer stop the machine.
            error:
              event.reason &&
              `Connection lost: ${event.reason}. If the machine is still moving, use its own stop or emergency stop.`,
          });
          log("Disconnected.");
          break;
      }
    });
  }

  return {
    ports: [],
    port: null,
    ...DISCONNECTED,
    console: [],
    error: null,
    originMoved: false,
    jobParts: [],
    jobAtc: false,

    startJob: async (programId, parts, atc) => {
      set({ jobParts: parts, jobAtc: atc, probe: null });
      await get().act({ type: "startJob", programId });
    },

    refreshPorts: async () => {
      try {
        const ports = await ipc.listPorts();
        const current = get().port;
        set({
          ports,
          port: ports.some((p) => p.name === current) ? current : (ports[0]?.name ?? null),
        });
      } catch (e) {
        set({ error: message(e) });
      }
    },

    setPort: (port) => set({ port }),

    connect: async () => {
      const { port } = get();
      if (!port) return;
      set({ connecting: true, error: null });
      try {
        await ipc.connectMachine(port);
        set({ connecting: false, connected: true });
        log(`Opened ${port}.`);
      } catch (e) {
        set({ connecting: false, error: message(e) });
      }
    },

    disconnect: async () => {
      try {
        await ipc.disconnectMachine();
      } catch (e) {
        set({ error: message(e) });
      }
    },

    act: async (action) => {
      set({ error: null });
      if (action.type === "line") log(`> ${action.text}`);
      try {
        await ipc.machineAction(action);
        if (movesWorkZero(action)) set({ originMoved: true });
      } catch (e) {
        set({ error: message(e) });
      }
    },
  };
});

/**
 * Whether an action may move the XY work zero: "Zero XY here", or a console
 * line that sets an offset (G10, G92, even typed without spaces), switches
 * to another set of zeros (G54 to G59), or clears them ($RST=# or $RST=*).
 * Only used to warn, so it errs on the side of saying yes.
 */
export function movesWorkZero(action: MachineAction): boolean {
  if (action.type === "setOrigin") return !action.z;
  if (action.type !== "line") return false;
  return /G0*(10|92)(?!\d)|G0*5[4-9](?!\d)|\$RST=[#*]/i.test(action.text);
}

export const jobIsActive =(job: JobProgress | null) => job?.state === "running";
