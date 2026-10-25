# Changing bits within one job (manual and automatic tool changes)

Status: built, steps 0 to 7, except the fixed tool-length setter in step 6 (1e). Tested against the simulated controller and by automated tests only: not yet run on any real machine, touch plate or tool changer, and the screens have not been clicked through in a running app. See "As built".

## Review changes

Revised 2026-10-24 after the `gcode-safety-reviewer` (verdict: needs changes) and `web-parity-checker` (5 gaps) reviews. The seven Decisions stand. What changed:

- **The plan now builds on machine control as built**: `check_can_move`, `SpindleMode`, the `$G` read after a job, `after_reset` and the resync. Machine control must merge first.
- **The grblHAL restart fix is now step 0.** It comes before any of the parts work.
- **Stricter spindle checks.** A speed that hasn't been reported is refused, and the spindle must be known to be off (`M5`). There is a spin-down dwell after `M5`. Each part before a change ends with `G4 P0.1`, and Kerf waits for a fresh Idle after it.
- **Stricter Z and X/Y checks.**
  - "Z is set" and "X/Y zero unchanged" are decided by reading `$#` (G54, G92 and the tool length offset).
  - Kerf's own zeroing uses `P0`.
  - Parts open with `G17 G21 G90 G94 G54`.
- **The probe was redesigned.**
  - The lines go one at a time, and `G10` is only sent after `[PRB:…:1]`.
  - The default travel is 10 mm, also capped by the room left in Z.
  - There is one rule for alarms (only Kerf's own probe alarms keep the job).
  - A clip-and-plate tick box comes before Continue.
- **Parts are cleared on any stop, reset or restart.** Start is refused while parts remain, and Zero XY is refused while waiting.
- **Automatic changer (ATC) jobs** start with `T<first> M6` and never send `G49`. Stopping during a change is listed as a hardware unknown.
- **Exported files carry clearer comments.** The change lift is checked against the room in Z, and the change card reads each part's stored bit.
- **Browser:**
  - `kerf-gcode` builds every saved text; nothing slices text.
  - The full program is kept on both sides.
  - The export signatures are listed.
  - One click saves one file, with no "Save all".
  - The README and Done-when checks for the browser are spelled out.
- **More hardware tests.**

## As built

Built 2026-10-24 on top of machine control as merged (`d032c5f`). Every test named under Safety exists and passes. Where it differs from the plan:

- **Built all at once.** The steps went in order, but in one change, so step 2's temporary "Start refuses multi-part jobs" was never shipped: step 3 replaced it in the same change.
- **The fixed tool-length setter (1e) is not built.** The plan marks it "later" and names no test for it. The fixed change place from step 6 (`G53` moves, homing this session, the travel check) is built. The README's "Not built yet" line now names the setter and grblHAL's own tool-change handshake.
- **Part texts are byte ranges, not copies.** `ProgramPart` keeps the byte range of its stream (and of its operations alone) inside `Program.text`, instead of a `stream: String`. That way a large relief isn't held three times over. `Program::part_stream(i)`, `part_streams()`, `part_file(i)` and `with_m6()` build the texts. `kerf_core::generate` now returns `(Generated, Program)`, splitting the text off with `Program::split_off_text()`. Both sides keep that whole `Program`: desktop `StoredProgram.program` and web `State.gcode`.
- **Extra `Program` fields:** `needs_homing`, `atc`, `change_lift` and `machine_moves`. The sender's `JobOptions` comes from these.
- **Sender API.** `Connection::start_job(&str, Tool)` is kept, so every existing test is unchanged. It now calls `start_parts(&[&str], Tool, &JobOptions)`, which replies through the worker, so the worker can refuse a start while parts remain.
- **Sender events.** The parts' progress comes in a new `Event::Parts { progress }` (part, of, waiting, spindleOff, zSet, plateUsed), alongside `JobState::WaitingForBit` on `Event::Job`. Changing `Event::Job`'s fields would have broken the existing tests. Probing reports through `Event::Probe` and `Event::ProbeTested`. `Event::Status` gains `pins`, and `Event::Welcome` gains `firmware`. `Event::Firmware` reports the answer to `$I`.
- **Zeroing goes through the worker.** `Connection::zero_xy()` and `zero_z()` send `G10 L20 P0 …`, and XY is refused there while a job waits. That makes the refusal testable against the simulated controller; `machine.rs` calls them.
- **Continue and Probe wait for a fresh `$#`.** The worker answers them once the `$#` it asked for comes back, within 1.5 s, which is less than the caller's 2 s timeout. A request that arrives or finishes later is refused rather than carried out. The `machine_action` command is synchronous, so the window can wait up to that long.
- **Probe defaults.** The probe uses 10 mm travel, 100 then 25 mm/min, a 2 mm back-off, a 3 mm slow travel, and a 5 mm lift after. The interface always sends the 10 mm travel. `plate_used` is set when probing starts, not when it ends, so a probe that stops after its `G10` still asks for the clip tick.
- **Probe alarm.** On `ALARM:4` or `ALARM:5` while probing, the job (and any parts) is kept, but the sent and queued manual lines are cleared, as any alarm clears them.
- **ATC needs the firmware known.** An ATC start with the firmware still unknown is refused with "Kerf is checking which firmware the machine runs". Kerf then sends `$I`, and the next Start can go ahead. `$I` isn't sent on connecting, because that would have changed the lines the existing tests pin.
- **Lift check.** It uses the fresh `$#` G54 Z, both at Start and at Continue. It assumes GRBL's usual negative machine space after homing (machine Z0 at the top).
- **Change-setting limits.** "Time for the spindle to stop" must be 1 to 120 s, and the lift 0 to 200 mm, checked when bits are named.
- **Part files.** Each starts with a `; Part n of N, with T…` comment. Non-last files end `M5`, `G4 P<spin_down>`, `G0 Z<lift>`, `G0 X0 Y0`, the end G-code, then "; Next: fit … then run part n". Bit names are cleaned before going into any comment (no brackets, semicolons or line breaks).
- **Laser jobs are untouched.** A laser job ignores the bit-change settings and any `tool`. The WeCreat preset's own end `M6` is pinned byte-for-byte by `a_vendor_m6_at_the_end_of_a_laser_job_is_left_alone`. `jobOrder` was not changed.
- **Assistant guard.** The tool-name test is in `src-tauri/src/assistant.rs` (`the_assistant_has_no_tool_that_reaches_the_machine`). It reads every `name: "…"` in `src/lib/assistantTools.ts`, and checks the file calls no machine or export function.
- **One existing test was made to wait.** `streams_a_job_to_completion_without_overflow` raced the `$G` sent after a job: it failed about 2 runs in 9 on the original code too. It now waits for that line before checking, and what it pins is unchanged. `parses_work_position_and_unknown_states` now uses `Cool` as its unknown state, since `Tool` is now a known one.
- **Fixes after the safety review (needs changes, 7 items):**
  1. *A part always travels before it plunges.* After a change, the writer marks its position unknown (`Writer::force_travel`), so the next part always writes its `G0 X… Y…`. Before this, a `G53` change could leave part 2 plunging at the change place. Tests: `machine_change_place_next_part_travels_before_plunging` (part 2 is exactly `G17 G21 G90 G94 G54`, `G0 Z5`, `M3 S12000`, `G4 P2`, `G0 X20 Y70`, `G1 Z-1 F200`, and its file travels before the first plunge) and `a_part_starting_at_the_origin_still_says_where_it_starts`.
  2. *The fixed change place is checked against the work.* `Program.change_clearance` (and `JobOptions.change_clearance`) is the larger of the lift and the job's highest safe height. At Start and at Continue, a fresh `$#` must show the place's Z at least that far above G54 Z. "Use the current position" in the Device step only works on a homed machine with Z within 5 mm of the top. Test: `machine_change_place_below_the_work_is_refused`.
  3. *G54 must be in use.* The worker tracks G54 from each `[GC:…]` reply, and asks again after a typed `G54`–`G59`. It refuses Probe, a multi-part Start and Continue unless G54 is in use. The probe's first line takes work Z from machine Z minus the fresh G54 Z, not the status report's work offset. Test: `multi_part_start_and_probe_refused_unless_g54`.
  4. *Hand-switched routers.* "Switch the router back on now, then Continue." only shows once Z is set (and the clip box is ticked, if the plate was used). The probe controls are hidden while it shows.
  5. *ATC.* `G0 Z<lift>` is written before the first `T<n> M6` too, pinned in `atc_job_starts_with_first_tool_change`.
  6. *Change place X/Y.* X and Y must be between minus the travel and 0, as Z must. Test: `change_place_xy_must_be_on_the_machine`.
  7. *Start reads the offsets first.* Any job with bit changes (more than one part, ATC, or a fixed place) reads `$#` before anything is sent. It refuses unless G92 and the tool length offset are zero; the lift and clearance checks use that read. Test: `multi_part_start_refused_with_g92_or_tool_offset_set`. The pinned line order in `waits_between_parts_until_continued` now starts `$$`, `$G`, `$#`. One-bit jobs start as before.
- **Fixes after the second safety review (2 items):**
  - *ATC changes travel before plunging too.* `force_travel` is now also set after the first `T<n> M6` and after every ATC change, so the next operation always writes `G0 X… Y…`. Test: `atc_next_operation_travels_after_m6`; after `T2 M6` the lines are exactly `G0 Z5`, `M3 S12000`, `G4 P2`, `G0 X20 Y70`, `G1 Z-1 F200`, and the same holds after `T1 M6` starting from the origin.
  - *Probe start height with `$10=0`.* The worker keeps the Z the last status report gave as it gave it: machine Z from `MPos`, or work Z from `WPos`. With `MPos`, work Z is machine Z minus the fresh G54 Z. With `WPos` only, work Z is used as reported, and machine Z (for the travel cap) is that plus the fresh G54 Z. A stored work offset that the reports haven't repeated lately is never used. Test: `probe_start_height_ignores_a_stale_work_offset` covers both report kinds, with a stale `WCO` in the reports.
  - The new `grblhal_banner_is_a_restart` test was also made exact about timing. Lines already on their way when the banner arrives may still reach the machine, so the test checks that nothing of the job is *sent* after Kerf has re-read the settings.
- **Not checked by hand.** The browser and desktop screens were type-checked and built, but not clicked through. Nor was the Done-when walk-through against the simulated controller done in the running app. The browser's saved files are checked by the kerf-web tests.

## The idea

This is for someone with a hobby CNC router: an X-Carve, a Shapeoko, a 3018, or a grblHAL machine. Most of these have one spindle, and the bit is swapped by hand. Today, a job whose operations use different bits is refused. The person has to untick operations, run one bit's work, change the bit, zero Z again and start the next part. Each step is easy to get wrong, and getting it wrong means cutting at the wrong height.

Afterwards, they can build one job that uses several bits. For example: carve a relief roughly with a 6 mm flat bit, finish it with a tapered ball nose, and cut it out with a 3 mm flat bit. Kerf runs the job and stops at each change. The spindle is off and the bit is raised. Kerf tells the person which bit to fit, helps them set Z again (with a touch plate or the paper method), and only carries on when they click. It refuses to carry on until Z has been set again, and the X and Y zero is never touched.

On a grblHAL machine with an automatic tool changer, Kerf writes `T<n> M6` and lets the controller do the change. Kerf only keeps track of which bit has which tool number.

## What Kerf already has

Read on `chore/dreamer-agent` (after `1863218`). The machine-control work was read in its worktree (`.claude/worktrees/agent-a728340cb9e403e8d`, branch `worktree-agent-a728340cb9e403e8d`), where it is built but not yet committed.

**Operations and bits**
- `src/state/tools.ts` (`useToolbox`) and `src/lib/tools.ts` (`Tool`: id, name, kind, diameter, flutes, cutLength, shank, angle, taperAngle). Bits have **no tool number** and **no length**.
- `src/state/project.ts`:
  - `OperationItem.toolId` picks a toolbox bit for each router operation. `withTool()` copies its diameter and shape into the operation and works out speeds.
  - `jobOrder()` runs the main artwork's operations first, then the added items', then cutouts.
  - **`usesMixedBits()` and `MIXED_BITS_MESSAGE`** refuse a job that uses more than one `toolId`. `generateGcode()` (line 1847) stops before generating, and `OperationsPanel.tsx` (lines 64, 114 and 149) shows the message and disables Generate.
  - `simpleSetup` avoids mixed bits on purpose (the comment at line 816).
- **`crates/kerf-gcode/src/job.rs` `Job::check()`** refuses operations whose `tool_diameter()` differ. The engine only knows a bit by its **diameter**, so a 3.175 mm flat bit and a 3.175 mm ball nose look alike to it. Only the interface's `toolId` check stops that pair today.

**G-code (`crates/kerf-gcode/src/generate.rs`, `generate_composed`)**
- The header is `; Generated by Kerf`, the start G-code, `G21`, `G90`.
- Each router operation (`cnc_profile`, `relief::carve` at relief.rs:225, `cutout::cut` at cutout.rs:253):
  - starts with `G0 Z<safe_z>`, `M3 S<rpm>` and `G4 P<SPINDLE_DWELL_S>`
  - ends back at `safe_z`
  - **does not switch the spindle off before the next operation.** The reviewer agreed that is fine within one bit.
- The job ends with `M5`, a travel back to the origin, and the end G-code.
- `Program.operation_starts` records where each operation begins.
- The 3D preview (`src/lib/simulation.ts` `tools?: BitShape[]`, Canvas.tsx:288) **already draws each operation with its own bit**.

**Sender, with machine control (`crates/kerf-grbl/src/connection.rs`)**
- **Moving.** Every Kerf-built move goes through `Command::Move` → `check_can_move()`. That check refuses when:
  - a `job` or `waiting_job` exists, or a reset is pending
  - the reported spindle speed is missing (`None`) or above 0
  - `SpindleMode` is `On` or `Unknown`
  - an `M3`/`M4`/`M5` is queued or in flight
  - a held jog, a resync or queued manual lines are outstanding
- **Spindle tracking.**
  - Typed lines set `SpindleMode` through `spindle_words()` when they are queued.
  - `[GC:…]` replies set it through `modes_spindle()`.
  - When a job finishes, the mode becomes `Unknown` and `$G` is queued, so Kerf asks rather than assumes.
- **Resets.** `after_reset()` clears `reset_pending`, `held` and `resync`, and sets the mode to `Unknown`. `abandon()` clears `job`, `waiting_job` and the manual queues. A `Welcome` runs both and queues `$$` and `$G`. Alarms call `stop_held` and `abandon`.
- **Status.** `Event::Status` carries `machine_position`, and Stop while homing resets straight away.
- **Typed lines.** Manual `Command::Line`s are refused while a job or waiting job exists.
- **Missing pieces:**
  - `response.rs` doesn't parse `Pn:` (probe pin), `[PRB:…]` (probe result) or `$#` replies (`[G54:…]`, `[G92:…]`, `[TLO:…]`)
  - there is no `Tool` state
  - **only the banner `Grbl ` is a welcome** (`response.rs:91`). grblHAL's `GrblHAL 1.1f …` banner isn't seen as a restart, so after a grblHAL reset Kerf neither clears its state nor reads the settings again.
- **`src-tauri/src/machine.rs`:**
  - `StartJob { program_id }` uses one `StoredProgram` text (`src-tauri/src/lib.rs`)
  - `SetOrigin` sends `G10 L20 P1 X0 Y0` and `G10 L20 P1 Z0`
- **`src/components/MachinePanel.tsx`:** Start takes two clicks (armed), and there are Pause, Resume, Stop, Zero XY here, Zero Z here, and the jog pad (now with held jogging).

**Export**
- Desktop: `export_gcode(suggested_name, program_id)` in `lib.rs` saves `StoredProgram.text`.
- Browser:
  - `src/web/files.ts` `exportGcode({ suggestedName })` saves the engine's `gcode_text`
  - `crates/kerf-web/src/lib.rs` `State.gcode` keeps **only the text**
  - `programId` is ignored
- `ipc.ts:186` `exportGcode(suggestedName, programId): Promise<string | null>`; `project.ts:1888` builds `<image name>.gcode`.

**Help:** `src/lib/help.ts:284` "Changing the bit" describes the untick-and-rerun routine; line 254 says "Unplug or switch off the router before changing a bit."

**Related ideas**
- *Machine control* ([machine-control.md](machine-control.md)) is a **prerequisite**. This plan reuses its spindle tracking and `check_can_move`.
- *A separate roughing pass for reliefs* (README "Not built yet") is not a prerequisite. Two relief operations already work as a crude rough-then-finish, because a relief never cuts below the surface.
- *Probing* was set aside in machine-control.md. This plan brings in the minimum: Z with a touch plate, one deliberate action at a time.

**What's missing:**
- tool numbers on bits
- a job split into parts by bit, and G-code at each change
- a sender that waits between parts and lets the person jog, probe and zero Z
- a backend check that Z was set again and X/Y wasn't touched
- export for other senders
- grblHAL detection, and the `Tool` state for automatic changers

## Approach

### How other software does it (briefly)

- **Plain GRBL 1.1 has no `M6`.**
  - It rejects `M6` as an unsupported command (`error:20`) and accepts and ignores a `T` word.
  - `M0` pauses in `Hold`, and while held GRBL won't jog or probe.
  - So on plain GRBL the change can't happen *inside* one streamed program. The sender must stop with the machine `Idle`.
- **bCNC** intercepts `M6` and runs its own macro: go to a change position, pause, probe at a fixed setter, adjust Z. It needs homing. ([bCNC Tool Change](https://github-wiki-see.page/m/vlachoudis/bCNC/wiki/Tool-Change), [fixed touch plate](https://github.com/vlachoudis/bCNC/wiki/Installing-a-fixed-Touchplate-for-tool-changes))
- **grblHAL** has `$341` modes for tool changes:
  - 0: fully manual
  - 1–3: touch-off variants
  - 4: ignore `M6`
  - ATC: "fully transparent to the sender"
  
  On `M6` it enters `Tool`, waits for `0xA3`, allows jogging, rejects other motion with `error:40`, and continues on cycle start. ([tool change](https://github.com/grblHAL/core/wiki/Manual,-semi-automatic-and-automatic-tool-change), [protocol](https://github.com/grblHAL/core/wiki/Manual-tool-change-protocol)) gSender users report getting stuck in `Tool` with no dialog ([Sienci forum](https://forum.sienci.com/t/no-dialog-box-on-tool-change/22070)).

### Chosen design: Kerf's sender stops *between* parts, with the machine Idle

The program is split into **parts, one per run of operations that share a bit** (compared by `ToolRef.number`, not diameter).
1. Kerf streams part 1 to its end. The part finishes with the spindle off, the bit raised and over the change position, and a `G4 P0.1` that only answers once motion has stopped.
2. Once a fresh `Idle` status arrives, the job **waits for a bit change**. No streamer is active, so jogging, Zero Z here and probing go through the ordinary `check_can_move` rules, on plain GRBL and grblHAL alike. No `M0` or `M6` is sent.
3. The next part is only streamed after a two-click **Continue**, and only if the backend's checks pass.

*Alternative considered:* one stream with `M0` (or `M6` with grblHAL `$341=1`). Rejected, because plain GRBL can't jog or probe while held, which is the stale-Z hazard itself. grblHAL's behaviour also depends on `$341`, the `0xA3` handshake and build dates Kerf can't check (Decision 6).

### 1. Manual changes (most hobby routers): the core

**1a. Tool identity in the engine**
- `Operation` (job.rs) gains `#[serde(default)] tool: Option<ToolRef>`:
  ```rust
  pub struct ToolRef {
      pub number: u32,       // fixed tool number, or 1, 2, 3… in order of first use
      pub name: String,      // "1 mm tapered ball nose"
      pub diameter: f64,
      pub shape: String,     // "Flat end mill", "Tapered ball nose" …, for the card
  }
  ```
- `Job::check()`:
  - different bits are allowed when **every** router operation carries a `tool`. Otherwise today's diameter check stays, so old callers and tests behave as before.
  - **New refusal:** the same `number` with a different diameter, shape or name, e.g. *"Operations 2 and 4 both say bit T2 but describe different bits."*
  - Laser jobs are unchanged.
- **The engine doesn't reorder (Decision 4).** The order stays `jobOrder`: main, items, cutouts. A change goes wherever two neighbouring operations have different `number`s.

**1b. G-code (`kerf-gcode`, new `src/tool_change.rs`, called from `generate_composed`)**

The engine builds **every text that is sent or saved**. Nobody else slices text.

```rust
pub struct ProgramPart {
    pub tool: Option<ToolRef>,
    pub operations: Range<usize>,
    /// What Kerf's sender streams for this part (see below).
    pub stream: String,
    /// Lines of `Program.text` this part covers, for progress display only.
    pub first_line: usize,
    pub line_count: usize,
}
impl Program {
    pub fn part_file(&self, i: usize) -> String;  // a standalone file for part i
    pub fn with_m6(&self) -> String;              // the single file with M6 changes
}
```

`Program.text` is the parts' `stream`s joined in order. For a **one-bit job there is one part, and `text` is byte-identical to today's** (pinned). That one part gets none of the changes below.

**Each part of a multi-bit job** is built like this:
- **The header** is `G17 G21 G90 G94 G54`. It never includes `G49`, so an automatic changer's length offset is left alone. Part 1 also has the `; Generated by Kerf` line and the start G-code first.
- **The body** is the operations, each with its existing `M3 S… / G4 P…` start. So **the spindle only restarts when the next part is streamed, after Continue**.
- **Every part except the last ends with the change block:**
  ```
  ; Bit change 1 of 2: fit T2 (1 mm tapered ball nose), then set Z again
  M5
  G4 P<spin_down>
  G0 Z<lift>
  G0 X<cx> Y<cy>
  G4 P0.1
  ```
  - `M5` comes first, before any move. `<spin_down>` comes from a Device setting, "Time for the spindle to stop" (default 5 s).
  - `<lift>` is in work coordinates: the larger of the job's highest `safe_z` and the "Raise for changing bits" setting (Decision 5: 20 mm).
  - `<cx> <cy>` is the change position:
    - **Over the work zero:** `X0 Y0` (Decision 1, the default).
    - **A fixed place on the machine** (offered when `$22=1`): `G53 G0 Z<mz>` then `G53 G0 X<mx> Y<my>`, replacing the two `G0` lines. Z always goes up first.
  - The final `G4 P0.1` is only answered once the planner is empty, which marks the end of the part.
- **The last part** ends as today: `M5`, back to the origin, the end G-code.
- **Bed check.** Work-coordinate change moves go through `Writer::travel`, so the bed-fit check covers them. `G53` moves are checked by the sender against `$130`–`$132`.

**`part_file(i)`**, a standalone file for other senders (Decision 3: the default way to save):
- the header, the start G-code, then `G17 G21 G90 G94 G54`
- parts 2 and later open with `; Fit T<n> (<name>) and set Z before running this file.`
- then the part, with the change lift, `M5`, back to the origin, and the end G-code
- a closing comment naming the next bit

**`with_m6()`**, a single file for senders that handle tool changes:
- the header lists every change: `; This job changes bits 2 times: T1 6 mm flat end mill, T2 1 mm tapered ball nose, T3 3 mm flat end mill.`
- at each change: `M5`, `G4 P<spin_down>`, the lift, the change position, `(Set Z again for T<n> before resuming)`, `T<n> M6`, then the next part's own `M3`/dwell

**1c. The sender (`kerf-grbl`, on top of machine control)**

*Starting*
- `Connection::start_job(parts: Vec<String>, tool: Tool, needs_homing: bool)`. A one-part job runs exactly as today.
- Part 1 becomes the `waiting_job` `Streamer` as now. The rest go into a new `pending: Option<PendingParts>`, **outside `job` and `waiting_job`**, so the existing checks see "no job" while the machine waits:
  ```rust
  struct PendingParts {
      parts: VecDeque<Streamer>, index: usize, of: usize,
      waiting: bool,                 // part finished, waiting for a bit change
      offsets_at_pause: Option<Offsets>, // G54, G92, TLO from $#
      z_set: bool, probing: Option<ProbeRun>, plate_used: bool,
  }
  ```
- `start_job` refuses while `pending` is `Some` (*"A job is waiting for a bit change. Continue or stop it first."*).
- `start_job` with `needs_homing` (the program has `G53` moves) is refused unless `homed_this_session`: a `Home` state followed by `Idle` since the connection opened, with no alarm or reset since.

*When a part ends*
- The part is over after its last `ok` (the `G4 P0.1`) **and a status report received after that `ok` reads `Idle`**.
- Machine control's existing job-finished path then runs: the spindle mode becomes `Unknown` and `$G` is queued. Kerf also queues `$#`. Once the `$#` reply is in, `offsets_at_pause` is recorded and `waiting = true`. `Event::Job { state: WaitingForBit { part, of } }` is emitted.
- The card only shows *"Kerf has switched the spindle off"* after `$G` has reported `M5` **and** a status reports `S0`.

*While waiting*
- Typed lines, jogs (including held jogs), Zero Z and probing are allowed, under the same `check_can_move` rules as at any other time.
- A typed `M3` sets `SpindleMode::On` as usual, and that blocks Continue.
- `machine.rs` refuses `SetOrigin { z: false }` (Zero XY) while waiting, and the panel disables it. A typed `G10 … X/Y` is still caught at Continue.
- **Kerf's own Zero Z here sends `G10 L20 P0 Z0`**, which sets the active system. (Zero XY here becomes `G10 L20 P0 X0 Y0` too, for consistency.)

*What ends the job and clears `pending`*
- Stop, Reset, `after_reset()`, a `Welcome` (from GRBL or grblHAL), Disconnect, and `abandon()`.
- **Any alarm**, except `ALARM:4` or `ALARM:5` raised while `probing` is `Some` (1d). Those keep the parts, leave `z_set` false and report the problem plainly. The `Alarm` handler checks this **before** calling `abandon()`.

*Z is set again* only when one of these is true:
- a `$#` read after the pause shows **G54 Z different from `offsets_at_pause`**. Kerf queues `$#` after every acknowledged line containing `G10` while waiting.
- or a Kerf probe completed (1d).

*`Connection::continue_job()`* runs the same checks as `check_can_move`, minus the job check:
- no reset pending
- the spindle reported, and reported 0
- `SpindleMode::Off`, with no `M3`/`M4`/`M5` queued or in flight
- no held jog, resync or queued lines

Then, from a **fresh `$#` read taken when Continue is pressed**:
1. `z_set` is true (*"Set Z again for the new bit before continuing: probe it, or lower it onto the material and press Zero Z here."*)
2. G54 X and Y match `offsets_at_pause` within 0.001 mm, G92 is all zero, and TLO is 0. Otherwise: *"The X/Y zero (or an offset) has changed since the job paused, so the next part would be cut in the wrong place. Stop the job."*
3. If the plate was used: the person ticked "Clip off the bit and plate moved away", **and** the latest `Pn:` doesn't show `P`
4. `$32` still suits a router
5. The lift check from 1f passes for the new Z zero

Only then does the next part become the `waiting_job`.

**1d. Setting Z with a touch plate (probe)**

`Connection::probe_z(plate, travel, fast, slow)` is allowed under `check_can_move`'s rules: before part 1, or while waiting. The worker runs it as a small sequence, **one line at a time**, each sent only after the previous line's reply:

*Before it starts* (refused otherwise):
- the touch-plate thickness is entered (Decision 7: blank by default)
- the probe test has passed this session: "Touch the plate to the bit; the light should turn on", showing `Pn:P` live
- `Pn:` doesn't show `P` right now
- `$#` shows G92 all zero and TLO 0

*Travel*
- **The person first jogs the tip to within about 10 mm of the plate.** The card says so.
- `travel` defaults to 10 mm. When the machine is homed and `$132` is known, it is also capped by the room left to the bottom of Z: machine Z + `$132`.
- If the plate was left out, nothing stops the bit except the travel limit. **The person is the one who notices the missing plate.** The card says this plainly: *"Put the plate under the bit before probing. Without it the bit goes down 10 mm into whatever is below."*

*Lines* (work coordinates, so nothing is left in `G91`; `wz` is the current work Z from status):
1. `G90 G38.2 Z<wz − travel> F<fast>` (default fast feed 100 mm/min), then wait for `[PRB:x,y,z:1]` and `ok`
2. `G90 G0 Z<prb_wz + 2>`
3. `G90 G38.2 Z<prb_wz + 2 − 3> F<slow>` (default slow feed 25 mm/min), then wait for `[PRB:…:1]`
4. **only after the second `[PRB:…:1]`:** `G10 L2 P0 Z<prb_mz − plate>`. `prb_mz` is the reported machine Z of that contact (GRBL reports `PRB` in machine coordinates).
5. `G90 G0 Z<plate + 5>`, then `$#` to confirm, and `z_set = true`, `plate_used = true`

*If it fails*
- A `[PRB:…:0]`, an error, or `ALARM:4`/`ALARM:5` stops the sequence. **No `G10` is sent**, `z_set` stays false, and the parts are kept.
- The message: *"The probe didn't touch (or was already touching). Unlock, check the clip and the plate, and try again."*

*Parsing* (`response.rs`):
- `Pn:` → `pins: Option<String>`
- `[PRB:x,y,z:1|0]` → `Response::Probe { position, ok }`
- `[G54:…]`, `[G92:…]` and `[TLO:…]` → `Response::Offset`
- The `[PRB:…]` line inside a `$#` reply is told apart because it arrives while a `$#` is in flight.

**1e. Fixed tool-length setter (later, needs homing)**
- **With the first bit, after zeroing on the material:** "Measure this bit on the setter" raises Z, goes to the setter's machine XY, probes (as 1d, without the plate) and stores `ref_mz`.
- **After each change:** probe again to get `new_mz`, then send `G10 L2 P0 Z<g54_z + (new_mz − ref_mz)>`. This moves only the Z zero.
- Kerf uses `G10 L2`, not `G43.1`, because a soft reset clears a tool length offset without telling anyone.

**1f. Interface**
- **Device step (router only), "Changing bits" box**
  - Settings that change the G-code are shown in **both versions**:
    - "How bits are changed": By hand / Automatic tool changer (grblHAL)
    - "Where to change them": Over the work zero / A fixed place on the machine. The second has a "Use the current position" button on the desktop. In the browser it is typed in, with the note "needs a machine that homes".
    - "Raise for changing bits" (mm)
    - "Time for the spindle to stop" (s)
  - Settings that only matter when Kerf runs the machine are **desktop only**. In the browser they're hidden, with the line *"Touch plate and router switch settings are in the desktop app, which runs the machine."*:
    - "Touch plate thickness" (mm, blank)
    - "My router is switched on and off by hand" (trim routers on the X-Carve and Shapeoko ignore `M3`/`M5`)
- **Toolbox:** an optional "Tool number" field.
- **Operations step**
  - Generating multi-bit router jobs is allowed: drop the `usesMixedBits` early return in `generateGcode()` and the Generate-button disable.
  - Show "Bits for this job": *"1. 6 mm flat end mill — Carve (rough). 2. 1 mm tapered ball nose — Carve (detail). 3. 3 mm flat end mill — Cut out. The job stops 2 times to change the bit."*
  - If a bit is used again after another one, add the hint to group them.
  - This list shows in both versions.
- **Machine panel (desktop)**
  - **Start.**
    - Before Start: *"Fit bit 1 (6 mm flat end mill) and set zero as usual."* While running: *"Part 1 of 3: 6 mm flat end mill"*.
    - The bit details shown come from the **parts snapshotted when the job started** (each part's `ToolRef`), not the live toolbox or a regenerated program.
  - **Lift check, at Start and at Continue.**
    - When homed and `$132` is known: refuse if work Z0 + lift would go above machine Z0 (`g54_z + lift > 0`). *"The bit can't rise 20 mm above the material here; there's only 12 mm left. Lower "Raise for changing bits" to 12 or less and generate again."*
    - Otherwise, the Start card warns: *"Kerf can't check there's room for the bit to rise 20 mm. Check by hand."*
    - Kerf doesn't quietly lower the lift, so the file that was previewed is the file that runs.
  - **The change card, at `WaitingForBit`:**
    1. *"Kerf has switched the spindle off; wait until it stops turning."* This only shows once `$G` says M5 and status says S0. Until then: *"Waiting for the machine to confirm the spindle is off…"*. If the router is switched by hand: *"Switch the router off at its own switch and unplug it."*
    2. *"Fit T2: 1 mm tapered ball nose."* from the part's `ToolRef`. Hold the router, not the gantry, when loosening the collet.
    3. *"Set Z again."* Either **Probe Z with the touch plate** (two clicks; the tip must be within 10 mm of the plate), or the jog pad and **Zero Z here**. *"Raise it more with the jog pad if you need room."* (Decision 5). A tick shows once the backend reports `z_set`.
    4. If the plate was used: a tick box, *"Clip off the bit and plate moved away."*
    5. If switched by hand: *"Switch the router back on now."*
    6. **Continue with bit 2.** Two clicks; the second reads **"New bit fitted and Z set?"**. It is disabled until 3 (and 4) are done. The backend checks again regardless.
    7. **Stop the job here.**
  - Zero XY here is disabled while waiting.
  - If `$1` isn't 255: *"The motors let go while the machine waits, so the head can be pushed out of place. Change the bit gently, or set $1=255."*

**1g. Export, in both versions**
- **One click saves one file.** There is no "Save all", because a zip would need a new dependency.
  - A job with changes shows **Save part 1 (6 mm flat end mill)…**, **Save part 2…** and so on.
  - **Save as one file with tool changes (M6)…** carries the warning *"Plain GRBL refuses M6. Use this only with a sender that stops for the bit change and lets you set Z again, such as bCNC, UGS, gSender or ioSender."*
- **The interface builds the suggested filename,** `<name>-<part>-<bit>.gcode` (e.g. `owl-1-6mm-flat-end-mill.gcode`). It is made safe for every file system: letters, digits, `.`, `-` and `_` only; others become `-`, repeats collapse, the ends are trimmed, and the whole name is cut to 100 characters.
- **Both sides keep the whole program.** Desktop `StoredProgram` and web `State` keep `text`, `parts` (with `stream` and `ToolRef`) and the means to build `part_file`/`with_m6`, rather than one text.
- **Signature changes:**
  - `src-tauri/src/lib.rs` `export_gcode(app, suggested_name, program_id, part: Option<usize>, with_tool_changes: bool, state) -> Result<Option<String>, String>`
  - `src/lib/ipc.ts` `exportGcode(suggestedName, programId, part?: number, withToolChanges?: boolean): Promise<string | null>`
  - `src/state/project.ts` `exportGcode(part?: number, withToolChanges?: boolean)` builds the filename and calls ipc
  - `src/web/files.ts` `exportGcode({ suggestedName, part, withToolChanges })` calls `engine("gcode_text", { part, withToolChanges })`. `programId` is ignored, as today. Returns `string | null`.
  - `crates/kerf-web/src/lib.rs` `gcode_text` takes `{ part?: number, withToolChanges?: boolean }`: none gives `text`, `part` gives `part_file(part)`, `withToolChanges` gives `with_m6()`

### 2. Automatic tool changers (grblHAL): narrow scope

**Kerf's job:** tool numbers, `T<n> M6`, and the controller's states. **The controller's job:** the change itself and any length measuring (its macro or plugin, `$341`, `G59.3`).
- **Firmware detection** (step 0): `GrblHAL <ver>` is a welcome, and `Event::Welcome` gains `firmware: Grbl | GrblHal`. Kerf also sends `$I` and reads `[FIRMWARE:grblHAL]`.
- **G-code (ATC mode only):**
  - **Every ATC job starts with `T<first> M6`** after the header, before the first `M3`.
  - At each change: `M5`, `G4 P<spin_down>`, `G0 Z<lift>`, `T<n> M6`, then the next operation's `M3`/dwell. There is no Kerf change position.
  - It is one stream, with no Kerf parts. `G49` is never written.
  - Every bit must have a tool number: the interface asks, and `Job::check` refuses without one.
- **Sender:**
  - It refuses ATC jobs unless the firmware is grblHAL.
  - `MachineState::Tool` is added, and counts as moving, so Stop shows. Stop resets straight away in `Tool`, as in `Home`.
  - **Kerf doesn't promise that stopping during an automatic change is safe.** The card says: *"Stopping during an automatic change can leave the tool half-released. Check the spindle by hand before running again."* That is hardware unknown 9.
  - It doesn't send `0xA3` in v1 (Decision 6). An unexpected `Tool` wait shows: *"The machine is waiting for a bit change that Kerf can't help with in this mode. Stop the job, or finish the change with the machine's own controls."*
- **Not in scope:** ATC macros, tool tables, stored length offsets, pocket positions.

### 3. Hardware a hobbyist could add (context for the software)

| Add-on | What it is | What the software must support |
|---|---|---|
| **Touch plate** (a metal plate and a clip on the bit) | The cheapest repeatable Z. Many machines ship with one (Shapeoko BitZero, X-Carve probe). | Plate thickness, the `Pn:P` test, bounded one-line-at-a-time `G38.2`, and the "clip off" tick: 1d. |
| **Fixed tool-length setter** | Measures each bit against the first, with no clip. Needs homing. | `G53` positions, a stored reference, `G10 L2` Z correction: 1e. |
| **ER collets with pre-set nuts / quick-change holders** | Quick swaps at a known stick-out. | Nothing new. Still set Z again (Decision 2). |
| **A fixed change spot** | Easier access to the collet, and a place for a setter. | "A fixed place on the machine", raising Z first, homing required: 1b, 1c. |
| **A simple tool rack** | Keeps the bits in order. | The tool number on each bit and on the card. |
| **DIY automatic changer** (a pneumatic ATC spindle such as ER11 or ISO20, a rack, an air valve, grblHAL with a macro or plugin) | Several hundred pounds or dollars, plus air and wiring. | Only part 2. |

## Changes, by area

- **Rust engine (`crates/...`)**
  - `kerf-gcode/src/job.rs`:
    - `ToolRef` and `Operation.tool`
    - the `Job::check` changes
    - `Machine.tool_change: ToolChange { mode: ByHand | Atc, place: OverWorkZero | Machine([f64; 3]), lift: f64, spin_down: f64 }`, with serde defaults: by hand, over the work zero, 20 mm, 5 s
  - new `kerf-gcode/src/tool_change.rs`: part splitting, part headers, the change block, `part_file`, `with_m6`, and the ATC lines
  - `generate.rs`: `Program.parts`; the one-part path unchanged
  - `kerf-grbl/src/response.rs`: the `GrblHAL` banner, `Pn:`, `[PRB:…:1|0]`, `$#` offsets, `MachineState::Tool`
  - `kerf-grbl/src/connection.rs`:
    - `PendingParts`, the end-of-part fresh-Idle rule, and the `$#` reads
    - `continue_job`, `probe_z` (sequence), and the alarm rule
    - clearing `pending` in `after_reset` and `abandon`
    - `homed_this_session`, Stop in `Tool`, `firmware`
  - new `kerf-grbl/src/probe.rs`: pure line builders, for pinned tests
  - `kerf-core`: return `parts` from `generate`
  - `kerf-web/src/lib.rs`: `State` keeps the whole program; `gcode_text { part, withToolChanges }`; a test for each
- **Desktop backend (`src-tauri/...`)**
  - `lib.rs`: `StoredProgram` keeps the whole program; `export_gcode` gains `part` and `with_tool_changes`
  - `machine.rs`:
    - `StartJob` passes the parts' `stream`s and `needs_homing`, and snapshots the `ToolRef`s
    - new `ContinueJob`, `ProbeZ { travel }`, `ProbeTest`
    - `SetOrigin` uses `P0`, and refuses `z: false` while waiting
  - No new names in `generate_handler!`.
- **Browser stand-in (`src/web/...`):** `files.exportGcode` forwards `part` and `withToolChanges`. `machine_action` stays `needsDesktop`. No new command names.
- **Interface**
  - `lib/tools.ts` and `Toolbox.tsx`: `number?: number`
  - `state/project.ts`:
    - the `toolChange` settings
    - drop the `usesMixedBits` early return in `generateGcode()`
    - send `tool` with each router operation
    - `bitsForJob()`
    - `exportGcode(part?, withToolChanges?)` and `safeFileName()`
  - `OperationsPanel.tsx`: the bit list, per-part save buttons and the M6 save; the Generate button no longer disabled by mixed bits
  - `DevicePanel.tsx`: the "Changing bits" box (desktop-only settings hidden in the browser)
  - `state/machine.ts`: `pins`, `firmware`, the `waitingForBit` state, `zSet`, the snapshotted parts
  - `MachinePanel.tsx`:
    - progress by part, and the change card with its ticks
    - the probe test, and two-click Continue
    - Zero XY disabled while waiting, and the lift check message
    - until step 3 ships, **Start refuses multi-part jobs** (also refused in `machine.rs`). This wording only exists in the desktop panel, so it never shows in the browser.
  - `lib/ipc.ts`: types and the new `exportGcode` signature
- **Help text / README**
  - `help.ts`:
    - rewrite "Changing the bit" (line 284)
    - add "Setting Z with a touch plate": the clip, the light test, jogging to within 10 mm, the plate thickness, taking the clip off afterwards
    - add the plate to the first-job checklist
  - `buttonTips.ts`: Continue, Probe Z, Measure on setter, the part save buttons
  - `README.md`:
    - in "Not built yet", replace "automatic tool changes" with what's left (the ATC is untested, and there is no grblHAL `0xA3` protocol)
    - under "It does not connect to a machine", add: *"You can plan a job with more than one bit and see where it stops to change them, and save it as one file per bit (or one file with M6). Running it, waiting at the changes and probing need the desktop app."*
    - in "Not yet tested", add bit changes and probing on real hardware

## Safety

This touches `kerf-gcode`, `kerf-grbl`, `machine.rs` and the machine panel. **All the CLAUDE.md rules apply, and `gcode-safety-reviewer` must run** on every step that touches them.

- **The spindle stays off during the change.**
  - `M5` is the first line of each change block, followed by `G4 P<spin_down>` before any move.
  - The card only says the spindle is off after `$G` reports M5 and status reports S0.
  - Continue needs the speed reported and 0, and `SpindleMode::Off`. A typed `M3` blocks it.
  - The next `M3` is inside the next part, so it only reaches the machine after Continue.
  - Hand-switched routers get the switch-off and unplug instruction, because `M5` can't stop them.
- **Never cut with a stale Z.**
  - Only a `$#` showing G54 Z changed since the pause, or a completed Kerf probe, sets `z_set`. The interface tick only mirrors it.
  - Every multi-bit part asserts `G54`, and Kerf's zeroing uses `P0`.
  - G92 and TLO must be zero to probe or continue.
  - Any reset, restart (GRBL or grblHAL), disconnect or non-probe alarm ends the job and clears the parts.
  - Start is refused while parts remain.
- **Keep the X/Y zero.**
  - Kerf never sends `G10` with X or Y during a wait; Zero XY is refused in `machine.rs` and disabled in the panel.
  - At Continue, a fresh `$#` must show G54 X/Y unchanged and G92 zero.
- **The end of a part is real.** A `G4 P0.1` ends each part, and the wait only starts after a status received after its `ok` says Idle.
- **Moves at a change.**
  - Z always goes up first, then across.
  - Jobs with `G53` moves need homing this session.
  - The lift is checked against the room left in Z when homed, and refused at Start or Continue if it doesn't fit; otherwise the person is warned.
  - Work-coordinate moves are bed-checked.
- **Probing is bounded.**
  - The person jogs the tip to within about 10 mm of the plate. The default travel is 10 mm, also capped by the room left in Z.
  - The feeds are slow, and the lines are sent one at a time. `G10` is only sent after `[PRB:…:1]`.
  - Probing is refused with the spindle on, with the probe already touching, before the light test, or with G92 or TLO set.
  - A missing plate is caught by the person, not by Kerf, and the card says so.
  - Before Continue, the "clip off" tick and a `Pn:` reading without `P` are required. Kerf never spins the spindle to test anything.
- **Controllers that can't pause.**
  - Kerf's sender never relies on the firmware pausing.
  - ATC jobs are refused on non-grblHAL firmware, and an unexpected `Tool` state shows Stop and a plain message.
  - The single-file `M6` export is optional and carries a warning, a list of changes in its header, and a comment before each `M6`. Per-bit files from part 2 on say to fit the bit and set Z first.
- **Every move is a deliberate click.** Continue takes two clicks ("New bit fitted and Z set?"), and Probe takes two clicks.
- **The assistant must never reach any of this.** No assistant tool calls `continue_job`, `probe_z` or `machine_action`, or saves files. Add a test that reads **every** tool name in `assistantTools.ts` (the same list `assistant_send` uses in both versions) and fails if any is a machine, probe, continue or export action.
- **Tests that pin exact lines.**
  - `kerf-gcode`:
    - `a_job_with_one_bit_is_unchanged` (byte-identical)
    - `a_bit_change_turns_the_spindle_off_then_lifts_then_moves` (`M5`, `G4 P5`, `G0 Z20`, `G0 X0 Y0`, `G4 P0.1`)
    - `multi_bit_parts_open_with_g17_g21_g90_g94_g54`, `no_part_writes_g49`
    - `the_next_part_starts_the_spindle_again_with_a_dwell`, `machine_change_place_raises_z_before_moving_across`
    - `the_same_number_for_different_bits_is_refused`, `splitting_compares_tool_numbers_not_diameters`
    - `part_files_from_two_on_say_fit_the_bit_and_set_z`, `every_part_file_ends_with_m5`
    - `m6_file_lists_every_change_and_says_set_z_before_each_m6`
    - `atc_job_starts_with_first_tool_change`, `atc_change_writes_t_and_m6_after_m5_and_lift`, `atc_needs_tool_numbers`
  - `kerf-grbl` (Sim):
    - `grblhal_banner_is_a_restart`, `grblhal_restart_while_waiting_ends_the_job`
    - `waits_between_parts_until_continued`, `waiting_card_not_shown_until_a_fresh_idle_after_last_ok`
    - `continue_is_refused_until_z_is_set`, `continue_is_refused_while_the_spindle_is_on`, `continue_refused_after_typed_m3_while_waiting`
    - `continue_is_refused_if_xy_zero_changed`, `continue_is_refused_with_g92_set`
    - `jogging_and_zero_z_work_while_waiting`, `zero_xy_is_refused_while_waiting`
    - `start_refused_while_waiting_for_bit`, `stop_while_waiting_ends_the_job`, `reset_while_waiting_clears_parts`
    - `probe_lines_are_exact_and_sent_one_at_a_time`, `probe_reply_success_flag_is_parsed`, `g10_not_sent_after_failed_probe`
    - `probe_is_refused_when_already_touching`, `probe_alarm_keeps_parts_other_alarm_ends_job`, `continue_refused_while_probe_pin_touching`
    - `machine_moves_need_homing_this_session`, `lift_refused_when_it_would_pass_machine_z0`
    - `atc_job_refused_on_plain_grbl`, `stop_in_tool_state_resets`
  - `kerf-web`: `gcode_text_with_part_returns_that_part_file`, `gcode_text_with_tool_changes_returns_the_m6_file`

## Hardware unknowns

Nothing Kerf sends has been proven on a real machine. All of these are on scrap, with the bit raised high for the first runs:

1. **Spin-down timing.** How long a Kerf-switched spindle (and a VFD) takes to stop after `M5`, and whether 5 s is enough. *Test:* an air job with two bits, Z zeroed 30 mm up; time from `M5` to standstill; check the card only appears after the dwell and reads S0.
2. **The end of a part after many short moves.** Is the fresh Idle after the `G4 P0.1` reliable with a full planner? *Test:* a part ending in a dense relief row; the card must not appear until motion has stopped.
3. **A probe with no contact** stops at the travel limit with `ALARM:5`, on GRBL 1.1 and grblHAL. Does the position and the modal state survive `$X`? *Test:* probe in the air, 10 mm travel; check it stops 10 mm down, the alarm is reported, no `G10` was sent, and the job is still waiting.
4. **The `[PRB:…:1]` format, and repeatability.** *Test:* check the reply format on each firmware; probe ten times onto the plate on scrap and check the Z values agree within 0.05 mm. Then cut a 0.5 mm pocket with bit 1, change bits, probe, cut a 0.5 mm pocket beside it, and measure that the depths match.
5. **Clip left on.** *Test:* finish a probe and leave the clip on and the plate under the bit. Continue must be refused while `Pn:` shows `P` and the tick is unticked. **Never spin the spindle to test this.**
6. **Reset or e-stop while waiting.** *Test:* at the change, press Kerf's Reset, then (separately) the machine's own e-stop or reset button. Each time the job must end and Start must need a new job.
7. **The lift on a short-Z machine** (a 3018). *Test:* homed, with Z zeroed low and a 20 mm lift; Start or Continue must refuse with the number that fits. Not homed: the warning shows, and the person checks by hand.
8. **The single-file `M6` export in other senders.** *Test:* air runs in gSender (each `$341` mode it offers), UGS and bCNC; note which stop properly and let the person set Z, and put the results in the help.
9. **ATC.** Whether the first-tool `M6` loads correctly, what the controller reports during the change, and what Stop mid-change does. *Test:* only with a tester who has one; an air job with two tool numbers, then the same job with Stop pressed mid-change and the spindle checked by hand.
10. **Steppers letting go (`$1`).** *Test:* mark the bit's spot on masking tape, change the bit at the pause, Continue, and check the next part's first plunge lands on the mark.
11. **`G53` change place and soft limits** on a homed machine. *Test:* a fixed place near the front; air job; Z rises fully before any X/Y move.
12. **Hand-switched routers:** do people reliably switch back on? *Test:* user feedback; add a "The router is running" tick if it's missed.

## Decisions

Decided 2026-10-24, going with the recommendations.

1. **Change position default.** Over the work zero (works without homing) or a fixed machine spot (better access, needs homing)? *Decided: over the work zero by default; the fixed place is offered when `$22=1`.*
2. **Allow "the bit length didn't change" (pre-set holders) to skip setting Z again?** *Decided: no in v1. Always set Z again. Revisit after hardware test 4 shows how close pre-set holders really are.*
3. **Default export for jobs with changes.** One file per bit, or one file with `M6`? *Decided: one file per bit by default, with `M6` as a labelled option. Per-bit files are safe with any sender.*
4. **Should Kerf reorder operations to group bits automatically?** *Decided: no. Keep the listed order (rough before finish matters), and show the hint to group them.*
5. **Raise height default (20 mm).** That's too much for some 3018s with short Z travel and long bits, and too little to swap a long bit on others. *Decided: 20 mm, editable, and the change card says "Raise it more with the jog pad if you need room".*
6. **grblHAL manual tool-change protocol (`0xA3`).** Support it, or always use Kerf's parts approach on grblHAL too? *Decided: parts approach everywhere for v1. It's simpler and tested the same way on both firmwares.*
7. **Touch plate default thickness.** Blank (no plate) until the person enters one, or a common value? *Decided: blank. A wrong default thickness is a wrong Z.*

## Build order

**Machine control ([machine-control.md](machine-control.md)) must be merged first.** Each step is shippable and checkable on its own.

0. **grblHAL restart detection.** In `response.rs`, `GrblHAL <ver>` becomes a welcome, with `firmware` on `Event::Welcome`. Test `grblhal_banner_is_a_restart`. This is a prerequisite of the parts work. `gcode-safety-reviewer`.
1. **Engine.** `ToolRef`, `Job::check`, `tool_change.rs`, `Program.parts`/`part_file`/`with_m6`, the kerf-gcode tests, and `kerf-web` keeping the whole program, with the `gcode_text` tests. The interface still refuses mixed bits, so nothing user-visible changes. Check with `cargo test --workspace`, `pnpm build:web` and the kerf-web tests. `gcode-safety-reviewer`.
2. **Generating and export in both versions.**
   - Drop the `generateGcode()` mixed-bits early return.
   - Add the bit list, tool numbers, the Device "Changing bits" box (desktop-only settings hidden in the browser), per-part saves, safe filenames, the `export_gcode` signature changes, and the README line.
   - The panel and `machine.rs` refuse to Start multi-part jobs.
   - *Now useful: split files for other senders.* `gcode-safety-reviewer`, `web-parity-checker`.
3. **The sender waits between parts** (desktop):
   - `PendingParts`, the fresh-Idle end of part, `$#` reads, and `continue_job` with all its checks
   - the change card with the paper method; Zero XY refused while waiting; `P0` zeroing
   - `homed_this_session`, the lift check, and the Sim tests
   
   The Start refusal from step 2 comes off. *Now the main idea works.* `gcode-safety-reviewer`.
4. **Touch plate.** `Pn:`/`PRB`/`$#` parsing, the light test, the `probe_z` sequence, the alarm rule, the clip tick and the `Pn` check at Continue, and the help text. Also usable before part 1. `gcode-safety-reviewer`.
5. **Single-file `M6` export** with its comments and warning. `gcode-safety-reviewer`, `web-parity-checker`.
6. **Fixed change place and fixed tool setter** (`G53`, `ref_mz`, `G10 L2 P0`). Needs homing. `gcode-safety-reviewer`.
7. **ATC mode.** `$I`, `MachineState::Tool`, `T<first> M6`, the changes, the refusals and the Stop-in-`Tool` card. Not done until a tester with an ATC has run hardware test 9. `gcode-safety-reviewer`.

## Done when

- CI passes: `pnpm build`, `pnpm build:web`, `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
- Every test named under Safety exists and passes, and `a_job_with_one_bit_is_unchanged` proves single-bit jobs are byte-identical.
- The reviewers named in each Build order step have signed it off.
- **By hand, desktop, against the simulated controller (or a real machine running an air cut):**
  - A relief made of two relief operations (6 mm flat, then a tapered ball) plus a 3 mm cutout lists 3 bits and 2 stops.
  - Start. After part 1 the card waits until the spindle is confirmed off, Z is up, and the head is over the work zero.
  - Continue is refused until Z is set.
  - Zero XY is disabled.
  - A typed `G10 L20 P0 X0` makes Continue refuse.
  - A typed `M3` makes Continue refuse until `M5`.
  - Probing (on hardware) sets Z; Continue then needs the clip tick; part 2 starts with `G17 G21 G90 G94 G54`, then `M3` and a dwell.
  - Stop or Reset at the change ends the job, and Start needs a new one.
- **By hand, browser:**
  - The bit list and the per-part save buttons show, and each saves one file with a safe name.
  - Every file starts with the header and `G17 G21 G90 G94 G54` and ends with `M5`.
  - The Device box shows only the settings that change the files, with the desktop-only line.
  - The machine panel's Start and probe controls aren't shown; the "does not connect to a machine" note covers them.
  - The desktop Start-refusal wording never appears.
- The assistant tool-name test passes.
