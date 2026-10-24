# Control the connected machine (X, Y, Z, homing)

Status: planned

## The idea

This is for a hobbyist who has connected Kerf's desktop app to their GRBL or grblHAL laser or router. Right now they can only move it in fixed steps by clicking. Afterwards they can hold a button (or an arrow key) to move smoothly until they let go, choose how fast it moves, send the head back to the work zero, see where it is from home as well as from the work zero, and get clear advice when homing is switched off or the machine starts locked. Every movement still needs a deliberate press by the person, the laser stays off, and the assistant can never reach any of it.

## What Kerf already has

The panel already covers more of this idea than its name suggests. Read after commit `c484729`:

- **`src/components/MachinePanel.tsx`**, section "Move":
  - jog pad X±/Y±/Z± (`JogButton`, one click = one step)
  - jog distance 0.1 / 1 / 10 / 100 mm (`Segmented`)
  - fixed speeds `JOG_FEED = 1500`, `JOG_FEED_Z = 300`
  - **Home**, **Unlock** (`$X`, enabled only in alarm), **Zero XY here** and **Zero Z here**
  - Section "Job": **Frame the job outline**, Start (two clicks), Pause/Resume/Stop
  - every move button is enabled only when `idle` (connected, state `idle`, no job running)
  - `moving` (which shows Stop) covers `run`, `jog` and `hold`, but **not `home`**
- **`src/state/machine.ts`**: `useMachine` keeps `state`, `position` (work position only), `settings` (all `$` settings by number) and `act(action)`. It does not keep the `spindle` value that the status event already carries.
- **`src-tauri/src/machine.rs`**, enum `MachineAction`:
  - `Jog` builds `$J=G91 G21 X… F…` inline
  - `CancelJog` exists (it sends `0x85`) but **no button uses it**
  - `Home` sends `$H`, `Unlock` sends `$X`
  - `SetOrigin` sends `G10 L20 P1 X0 Y0` or `G10 L20 P1 Z0`
  - `Frame` sends five `$J=G90 G21` corner moves
  - all of these go through the one `machine_action` command
- **`crates/kerf-grbl/src/connection.rs`**:
  - `Connection` has `send_line`, `hold`, `resume`, `cancel_jog`, `reset`, `start_job`, `stop_job` and `state()`
  - the worker thread (`Worker::serve`) polls `?` every 200 ms, queues manual lines (`manual_queue` and `manual_in_flight`, bounded by `RX_BUFFER_BYTES = 127`), and refuses manual lines while a job runs
  - `on_status` turns `MPos − WCO` into a work position and **drops the machine position** before it emits `Event::Status`
  - `StopJob` sends a feed hold, then resets once the machine reports `Hold:0` or `Idle`, or after `STOP_TIMEOUT` (3 s)
- **`crates/kerf-grbl/src/response.rs`**: `Status` already parses `MPos`, `WPos`, `WCO`, feed and spindle.
- **Browser**: `src/web/core.ts` already answers `connect_machine` and `machine_action` with `needsDesktop("Controlling a machine")`, and the panel shows a "does not connect to a machine" note. `README.md` already lists "It does not connect to a machine".
- **Assistant**: `src/lib/assistantTools.ts` has no machine tools (line 14 says why).
- **Help**: `src/lib/help.ts` ("Set zero", the first-job checklist) and `src/lib/buttonTips.ts` (Home, Unlock, Zero XY/Z, Frame).

So jogging, homing, unlocking (`$X`), Zero Z and a work-position readout are **already built**. These are the gaps worth filling:

1. **Hold to move.** You can't move continuously while a button or key is held. Long moves mean many clicks, or a 100 mm step that can't be stopped short except with Stop.
2. **Keyboard jogging.** There is none.
3. **Jog speed.** It is fixed at 1500 mm/min, which is too fast for lining up on a mark and too slow to cross a 1 m router bed.
4. **Go to work zero.** There is no button to send the head back to X0 Y0 to check where the origin is. Framing does something similar, but only once a program exists.
5. **Machine position.** Only the work position is shown, so after homing the person can't see where they are on the bed.
6. **Homing switched off (`$22=0`).** Home is still clickable, and GRBL answers `error:5` in the console. A machine that starts in alarm gets no advice on whether to Home or Unlock.
7. **Stop during homing (a safety gap).** While the state is `home`, the panel shows no Stop button, because `moving` leaves out `home`. In the backend, Stop sends a feed hold, which GRBL ignores while homing, so the soft reset only goes out once homing finishes or the 3 s timeout runs out. Only a soft reset stops homing.
8. **No laser check before jogging.** A `M3 S…` typed in the console leaves the spindle or laser on, and the jog pad will then move with it on. The status report's spindle value (`FS:…,S`) is parsed, but nothing checks it.

Not worth planning:
- **Go to a typed position.** Rarely needed by hobbyists, and the console already takes `$J=G90 X… Y… F…`.
- **Saved positions or work offsets G55–G59.** Kerf only uses G54.
- **Probing.** Separate idea, needs a probe plate and its own safety review.

## Approach

### Hold to move: short jog steps sent while the person holds, with a backend watchdog

This follows the method GRBL's own jogging guide recommends ([Grbl v1.1 Jogging](https://github.com/gnea/grbl/wiki/Grbl-v1.1-Jogging)), and bCNC and UGS do much the same:

- While the button is held, send a stream of very short `$J=G91` moves.
- On release, send jog cancel (`0x85`), which empties GRBL's queued jog moves and returns it to Idle without a reset.

The stream of short moves matters for safety. If the USB cable is pulled or Kerf crashes, the machine only finishes the few short moves it already has (well under a second of travel), instead of a long move running on to the end of the axis.

Ownership is split in two:

- **The backend worker owns the stream.** The interface only says "start", "still held" and "stop".
- **A watchdog in the worker** stops the move if "still held" stops arriving. That covers a frozen interface, a lost keyup or a hung webview, because the worker is its own thread.

Steps:

1. **Jog line building moves into `kerf-grbl`** as a new module `crates/kerf-grbl/src/jog.rs`, with pure functions whose output tests can pin exactly:
   - `step_line(dx, dy, dz, feed) -> String`: the existing `$J=G91 G21 X… F…`. `machine.rs` calls this instead of building the line itself.
   - `go_to_xy_zero_line(feed) -> String`: `$J=G90 G21 X0.000 Y0.000 F…`. It moves X and Y only and never Z.
   - `held_segment(direction, feed, dt) -> String`: one short move of `feed/60 × dt` mm along the held direction.
   - `segment_time(feed, accel) -> f64`: the dt from GRBL's guide, the larger of 0.05 s and `1.2 × v² / (2 a (N−1))` with N = 15, capped at 0.2 s. `accel` is the smallest of `$120`–`$122` for the moving axes. If those settings are unknown, held jogging is refused.
2. **`Connection::start_held_jog(direction, feed)`, `keep_held_jog()`, `stop_held_jog()`** each send a new `Command` to the worker. `direction` is `[i8; 3]`, each −1, 0 or 1. **Only one axis may move at a time.** Diagonals are left out of v1 so that a stuck key can only ever drive one axis. The worker:
   - **refuses to start** unless the state is `Idle`, no job is running or waiting, the manual queue is empty, and the last status reported spindle speed 0 (see step 4)
   - while held, **sends one segment each time the lead budget allows**: segments sent × dt − time since start ≤ 0.3 s. That keeps the queued motion small even when acceleration makes the machine lag. The RX limit of 127 bytes still applies.
   - **stops** (sends `0x85`, then sends no more segments) when any of these happens:
     - `stop_held_jog` arrives
     - no `keep_held_jog` has arrived for 300 ms (the watchdog)
     - a segment is answered with `error:` (for example soft limits, `error:15`)
     - an alarm arrives
     - the state is anything other than `Idle` or `Jog`
     - a job is asked to start
     - the connection is closing (`shut_down`)
   - **after a cancel, resynchronises its line count.** GRBL's guide doesn't say whether purged jog lines still get an `ok`. So the worker sends `G4 P0`, then on the `ok` for it empties `manual_in_flight` before accepting more manual lines. The tests cover a simulator that answers purged lines and one that doesn't.
3. **The interface** (`src/components/MachinePanel.tsx`):
   - The distance selector gets a fifth choice, **"Hold"** (0.1, 1, 10, 100, Hold). In Hold mode the pad moves only while it is pressed. The mode is visible, so a click never surprises anyone by moving further than expected.
   - In Hold mode `JogButton` uses pointer events:
     - `pointerdown`: `setPointerCapture`, then the `holdJogStart` action, plus an interval sending `holdJogAlive` every 100 ms
     - `pointerup`, `pointercancel` or `lostpointercapture`: `holdJogStop`
     - `contextmenu` is prevented, so a long touch doesn't open a menu
   - **A single `useHeldJog` hook** runs every stop path:
     - the window's `blur` event
     - `visibilitychange` to hidden
     - the panel unmounting (switching step)
     - `m.connected` turning false
     - `m.state` leaving `idle` or `jog`
     - `m.error` being set
4. **Laser and spindle check** (all jogs, Go to zero and Frame):
   - The worker keeps the last reported spindle value.
   - `Connection` refuses jog, held-jog, go-to and frame lines when that value is above 0, with: *"The laser or spindle is on (S…). Send M5 from the console before moving the machine."*
   - **The check lives in `kerf-grbl`, not only in the interface,** so a future caller can't skip it.
   - Typed console lines are not checked; they stay the person's own commands.
5. **Jog speed**:
   - a `Segmented` "Speed" control: Slow / Normal / Fast = 300 / 1000 / 3000 mm/min for X and Y, with Z at one fifth of those
   - each speed is capped at the machine's `$110`–`$112` maximum rates when they are known
   - the choice is remembered in localStorage (key `kerf.jogSpeed`), the same way `kerf.device` is
   - Hold mode uses the same speed, through `segment_time`
6. **Keyboard jogging, only while the jog pad has focus:**
   - The pad becomes one focusable group (`tabIndex=0`, role `group`, label "Jog pad: arrow keys move X and Y, Page Up and Page Down move Z").
   - Arrow keys move X and Y; Page Up and Page Down move Z. In step mode each fresh `keydown` (`e.repeat` false) is one step. In Hold mode `keydown` starts and `keyup` stops; repeats only count as "still held".
   - `Escape` stops a held jog, and otherwise sends `cancelJog` if the state is `jog`.
   - `preventDefault` stops the arrow keys scrolling the panel.
   - Clicking anywhere else, or switching window, blurs the pad, and blurring stops the move.
   - A visible focus ring and the label "Keys move the machine" show while it is focused.
   - **Keys are never caught globally,** because the arrow keys belong to the canvas and to text fields elsewhere.
7. **Go to XY zero** button in "Move":
   - one click sends `go_to_xy_zero_line` at the chosen speed (Fast capped at 3000)
   - it moves at the current height; the existing framing advice is repeated in its tip ("raise the bit clear first")
   - enabled only when `idle`; Stop shows while it moves (state `jog`)
8. **Position readout**:
   - `Event::Status` gains `machine_position: Option<[f64; 3]>`, computed as `MPos`, or `WPos + WCO` when only `WPos` is reported
   - the panel shows **"From work zero"** (today's `position`) and **"From home"** (machine position)
   - "From home" carries a note "(after homing)" when `$22=1`, and is hidden when `$22=0`, because then the number means nothing
9. **Homing switched off and starting locked**:
   - When `settings[22] === "0"`, Home is disabled with the tip *"Homing is switched off on this machine ($22=0)."*
   - In alarm, a short line explains the choice: *"The machine is locked. Home it so it knows where it is (or Unlock if it has no homing switches and you know where it is)."*
   - Unlock's tip gains *"Only if you know where the head is: the machine will not stop at the edges until it is homed."*
10. **Stop during homing**:
    - `moving` includes `home` (and `door`), so Stop shows.
    - In `Worker::handle`, `StopJob` while the last state is `Home` sends `0x18` (soft reset) straight away instead of a feed hold, because GRBL ignores a hold while homing.
    - GRBL then raises an alarm (homing aborted), which the existing alarm handling reports.

*Alternative considered:* one long jog clipped to the machine's travel, cancelled on release. That is fewer lines, but if the cancel byte is lost (cable pulled, app frozen) the machine keeps going to the end of the axis. Clipping it needs a homed machine with trustworthy `$130`–`$132`, and the backend watchdog would only help while the port is still open. Not chosen.

## Changes, by area

- **Rust engine (`crates/kerf-grbl`)**:
  - new `src/jog.rs` (line builders, `segment_time`), exported from `lib.rs`
  - `connection.rs`: new `Command::HeldJogStart / HeldJogAlive / HeldJogStop`; held-jog state in `Worker`; the watchdog and lead budget in `serve`; the resync after cancel; the spindle check; reset straight away for Stop while homing; `machine_position` on `Event::Status`
  - Sim tests (below)
  - no change to `kerf-gcode` or `kerf-make`
- **Desktop backend (`src-tauri/src/machine.rs`)**:
  - new `MachineAction` variants `HoldJogStart { x, y, z, feed }`, `HoldJogAlive`, `HoldJogStop` and `GoToXyZero { feed }`
  - `Jog` and `Frame` call the `kerf_grbl::jog` builders
  - `finite` and feed checks as today; `HoldJogStart` refuses more than one non-zero axis
  - no new Tauri command, so `lib.rs` `generate_handler!` doesn't change
- **Browser stand-in (`src/web/core.ts`)**: nothing to add. Every new action goes through `machine_action`, which is already `needsDesktop("Controlling a machine")`, and the panel already shows its browser note. The `web-parity-checker` should still confirm this.
- **Interface**:
  - `src/lib/ipc.ts`: new `MachineAction` members; `machinePosition` on the status event
  - `src/state/machine.ts`: keep `machinePosition` and `spindle`; clear both in `DISCONNECTED`
  - `src/components/MachinePanel.tsx`: the Hold choice, the Speed selector, the focusable jog pad with keys, a `useHeldJog` hook (could live in `src/lib/heldJog.ts`), Go to XY zero, the two-line position readout, the `$22` handling, alarm advice, and Stop shown during `home` and `door`
  - `src/lib/buttonTips.ts`: tips for Go to XY zero, Hold and the jog pad, and the revised Unlock tip
- **Help text / README**:
  - `src/lib/help.ts` "Set zero": "Choose Hold to move the bit while you hold the button or an arrow key; let go to stop."
  - first-job checklist: "Press Go to XY zero to check where the origin is."
  - README line 44: "jog (in steps, or while you hold a button or arrow key), home, set zero, go back to zero, frame, start, pause and stop"
  - the "What the browser version leaves out" list is unchanged; it already covers machine control

## Safety

This touches `kerf-grbl`, `src-tauri/src/machine.rs` and the machine panel, so every rule in CLAUDE.md applies.

- **Deliberate press only.** Every movement starts from a pointer press or a key press on a focused pad in the Machine panel. Held motion needs the press to continue, through the interface keep-alive and the backend watchdog together. Nothing moves on page load, on reconnect, or from a timer that starts by itself.
- **The assistant never reaches it.** No new Tauri command. The new `MachineAction` variants go only through `machine_action`, and nothing is added to `src/lib/assistantTools.ts`. The keyboard handler sits on the pad element, not on `window`, so nothing outside the panel can drive it. The reviewer should confirm that `assistantTools.ts` and `kerf-ai` still have no path to `machineAction`, `machine_action` or `kerf_grbl`.
- **Laser off.** `$J=` jog lines can't carry `M` or `S` words (GRBL rejects them), so a jog can't turn the laser on. Kerf also refuses to jog while the last status reports spindle or laser speed above 0. Framing gets the same check.
- **Bounded motion.**
  - held jog: one axis only, at most about 0.3 s of queued motion plus whatever the planner holds (at most 15 segments of ≤ 0.2 s)
  - Go to XY zero: one move to the origin the person set
  - with soft limits on, GRBL refuses moves past the edge, and that error ends the hold
- **Units and origin.** Every jog line carries `G21`. Steps and held segments are `G91`, so the modal state isn't touched. Go to XY zero is `G90` in work coordinates (G54), and moves X and Y only, never Z.
- **Stopping.**
  - jogs stop with `0x85`, never a feed hold, because GRBL's guide warns that a hold arriving after Idle suspends the machine
  - Stop during homing becomes an immediate soft reset
  - closing the connection mid-hold still resets through `shut_down`
- **Tests to add** (`cargo test -p kerf-grbl`), pinning exact lines and bytes:
  - `jog.rs`: `step_line(0, 1, 0, 1000) == "$J=G91 G21 Y1.000 F1000"`; `go_to_xy_zero_line(3000) == "$J=G90 G21 X0.000 Y0.000 F3000"`, with no `Z`; `held_segment` lengths and formatting; `segment_time` floors, caps and the formula
  - Sim tests in `connection.rs`:
    - `held_jog_sends_segments_only_while_kept_alive`
    - `held_jog_stops_with_jog_cancel_when_keep_alive_stops` (watchdog: `0x85` written, no further `$J`)
    - `held_jog_stop_sends_jog_cancel_not_feed_hold`
    - `held_jog_lead_never_exceeds_budget`
    - `held_jog_refused_unless_idle`, `held_jog_refused_during_job`, `held_jog_refused_with_spindle_on` (Sim reports `FS:0,100`)
    - `held_jog_ends_on_error_or_alarm`
    - `held_jog_rejects_two_axes` (in `machine.rs`, or as a `Connection` check)
    - `manual_lines_work_after_cancel_whether_purged_lines_are_acknowledged_or_not` (two Sim variants)
    - `stop_while_homing_resets_immediately`
    - `status_reports_machine_position`
  - The Sim needs a `Jog` state and `0x85` handling (empty `buffered`, go to Idle), plus an option to answer purged lines.
- **Reviewer agents:** run `gcode-safety-reviewer` on the `kerf-grbl` and `machine.rs` changes, and `web-parity-checker` on the interface and ipc changes.

### Hardware unknowns (only a real machine can settle these)

Do each on scrap with the laser off (or the router's spindle off and the bit raised), with a hand on the machine's own stop:

1. **How far it coasts after release.** Hold X+ at Fast and let go over a ruler taped to the bed. Note the distance past the release point. Expect under 0.3 s of travel; anything over 10 mm at Fast means the lead budget is too large.
2. **What happens when the cable is pulled mid-hold.** Hold at Slow, pull the USB cable, and measure how far it carries on. It must stop by itself within about a second.
3. **Whether purged jog lines get an `ok`.** Watch the console after a cancel. Then type `$$` and check it still answers, which shows the resync works.
4. **Laser stays off while jogging.** On a laser with `$32=1`: send `M3 S0`, then try to jog. It should go through (S is 0). Then send `M3 S10`. Kerf must refuse to jog and show the message. Never test with the laser actually firing.
5. **Soft limits.** On a homed machine with `$20=1`, hold towards an edge. It must stop at the edge with a clear message, not an alarm.
6. **Stopping homing.** Press Home, then Stop part way. It must halt at once and show the homing alarm, and Unlock or Home must recover it.
7. **Keyboard on each platform.** In WebView2 (Windows) and WebKitGTK (Linux, never run), check how auto-repeat behaves, and that Alt-Tab during a held arrow key stops the move.
8. **grblHAL.** Repeat 1, 3 and 6 on a grblHAL board, which may buffer differently.

## Dependencies

None. It also doesn't block ideas 1 and 2. If [auto-centre](README.md#2-auto-centre-the-design-on-the-material) (idea 2) later wants "move the head to the material's corner", it should reuse `kerf_grbl::jog` and the same deliberate-press rules.

## Open questions

1. **How Hold is chosen.** As a fifth option in the distance selector, or a separate "Hold to move" checkbox? *Recommend the selector option.* It is one control, and the current mode is always visible.
2. **Speed values.** Slow / Normal / Fast = 300 / 1000 / 3000 mm/min for X and Y, Z at one fifth, capped by `$110`–`$112`? *Recommend yes.* A small laser may want Fast lower; the cap handles most of that.
3. **Keys.** Arrows for X and Y and Page Up / Page Down for Z, only while the pad is focused? *Recommend yes.* LightBurn and UGS use similar keys, and focus-only avoids clashing with the canvas.
4. **Go to XY zero on a router.** Should it raise Z by a set amount first? *Recommend no for v1.* Kerf can't know a safe height without a homed Z, and the button tip tells the person to raise the bit. Revisit with probing.
5. **Diagonal held jogs.** Allow them (two keys at once)? *Recommend no for v1.* With one axis at a time, a stuck key can only drive one axis. Diagonal steps by clicking aren't offered today either.

## Build order

1. **Stop during homing.** Show Stop in `home` and `door`, and send a reset straight away while homing. Add `stop_while_homing_resets_immediately`. A small safety fix that ships alone.
2. **Jog lines into `kerf-grbl::jog`, plus the spindle check.** `machine.rs` uses the builders. Pin them with tests. Refuse moves while the spindle is on.
3. **Speed selector and Go to XY zero.** Interface plus one action, tips and help.
4. **Position readout and `$22` handling.** `machine_position` on the status event, the two-line readout, the Home and Unlock advice.
5. **Held jog in the backend.** Commands, watchdog, lead budget and resync, with all the Sim tests. No interface yet.
6. **Held jog in the interface.** The Hold choice, the pointer handling and the `useHeldJog` stop paths.
7. **Keyboard on the focused pad.**
8. **README and help text,** then `gcode-safety-reviewer` and `web-parity-checker`.

## Done when

- The CI commands from CLAUDE.md pass:
  - `pnpm build`
  - `pnpm build:web`
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`
- The `jog.rs` line tests and every Sim test named under Safety exist and pass.
- `gcode-safety-reviewer` and `web-parity-checker` report no unresolved findings.
- By hand on the desktop with no machine: the panel builds, Hold and Speed are remembered, and the pad shows its focus ring and label.
- By hand in the browser build: the Machine step still shows the desktop-only note, and nothing new is reachable.
- By hand on a machine: the hardware checks 1–7 above are done on scrap, and the results are recorded in the plan or an issue. Until then the README warning about unproven hardware stays as it is.
