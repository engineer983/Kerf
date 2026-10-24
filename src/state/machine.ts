import { create } from "zustand";
import * as ipc from "../lib/ipc";
import type { JobState, MachineAction, MachineState, PortInfo } from "../lib/ipc";

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
  state: MachineState | null;
  /** Work position in millimetres. */
  position: [number, number, number] | null;
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
  state: null,
  position: null,
  settings: {},
  job: null,
};

export const useMachine = create<MachineStore>((set, get) => {
  const log = (line: string) =>
    set((s) => ({ console: [...s.console, line].slice(-CONSOLE_LIMIT) }));

  if (ipc.isTauri()) {
    void ipc.onMachineEvent((event) => {
      switch (event.type) {
        case "status":
          set({ state: event.state, position: event.position });
          break;
        case "welcome":
          set({ version: event.version, settings: {} });
          log(`Connected to Grbl ${event.version}`);
          break;
        case "setting":
          set((s) => ({ settings: { ...s.settings, [event.number]: event.value } }));
          break;
        case "line":
          log(event.text);
          break;
        case "commandError":
          log(`${event.line} → error ${event.code}: ${event.description}`);
          break;
        case "alarm":
          log(`ALARM ${event.code}: ${event.description}`);
          set({ error: `Alarm ${event.code}: ${event.description}` });
          break;
        case "job":
          set({ job: event });
          if (event.state !== "running") {
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
