import { useCallback, useEffect, useRef, useState } from "react";
import * as ipc from "./ipc";
import { useMachine } from "../state/machine";

/** How often the interface says the button or key is still held. The
 * backend stops the move if it hears nothing for 300 ms. */
const ALIVE_MS = 100;

/** −1, 0 or 1 for each of X, Y and Z, with only one not 0. */
export type JogDirection = [number, number, number];

/**
 * Moves the machine while a button or key is held. Every way the hold can
 * end runs through `stop`: letting go (the caller), the window losing focus
 * or being hidden, any pointer being released anywhere, the panel closing,
 * the connection going, an error, or the machine leaving Idle or Jog. The
 * backend's own watchdog stops it too if this stops saying "still held".
 */
export function useHeldJog() {
  const timer = useRef<number | null>(null);
  const [active, setActive] = useState(false);
  const connected = useMachine((s) => s.connected);
  const state = useMachine((s) => s.state);
  const error = useMachine((s) => s.error);

  const stop = useCallback(() => {
    if (timer.current === null) return;
    window.clearInterval(timer.current);
    timer.current = null;
    setActive(false);
    // Sent directly, not through `act`, so a message saying why the move
    // ended (a refused move, say) is not cleared by letting go.
    void ipc.machineAction({ type: "holdJogStop" }).catch(() => undefined);
  }, []);

  const start = useCallback(
    ([x, y, z]: JogDirection, feed: number) => {
      // One axis at a time: a second press while one is held does nothing.
      if (timer.current !== null) return;
      timer.current = window.setInterval(() => {
        ipc.machineAction({ type: "holdJogAlive" }).catch(stop);
      }, ALIVE_MS);
      setActive(true);
      void useMachine.getState().act({ type: "holdJogStart", x, y, z, feed });
    },
    [stop],
  );

  useEffect(() => {
    if (!connected || error || (state !== null && state !== "idle" && state !== "jog")) stop();
  }, [connected, state, error, stop]);

  useEffect(() => {
    const hidden = () => {
      if (document.visibilityState === "hidden") stop();
    };
    window.addEventListener("blur", stop);
    window.addEventListener("pointerup", stop);
    window.addEventListener("pointercancel", stop);
    document.addEventListener("visibilitychange", hidden);
    return () => {
      window.removeEventListener("blur", stop);
      window.removeEventListener("pointerup", stop);
      window.removeEventListener("pointercancel", stop);
      document.removeEventListener("visibilitychange", hidden);
      // The panel is closing (another step was chosen).
      stop();
    };
  }, [stop]);

  return { active, start, stop };
}
