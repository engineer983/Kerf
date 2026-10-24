# Auto-centre the design on the material

Status: planned

## The idea

For anyone whose piece of material isn't pushed into the bed's origin corner, like an offcut dropped somewhere on the honeycomb or a board clamped in the middle of a router bed. Kerf finds the material in the camera picture, puts the design in the middle of it, and then lets the person correct the outline and nudge the design by hand. The person's words: "auto centering of the object within the detected material, with the ability to adjust."

Kerf can't do this today, because it assumes the material always sits in the origin corner. So the plan has two halves:
1. **The material can sit anywhere on the bed**, typed in or dragged. This works with no camera at all.
2. **The camera finds it.** This depends on idea 1, [camera-view.md](camera-view.md).

## What Kerf already has

- **The material only has a size, not a place.** `stockWidth` and `stockLength` live in `src/state/project.ts`. `stockRect()` (line 589) always returns `x: 0` and the origin-side edge. `MaterialPanel.tsx` says "The material sits in the front-left corner of the bed."
- **Centring on the material already exists, but it assumes that corner:**
  - `centred()` (`project.ts` line 971) runs when a picture, drawing or design is loaded, and when an item is added (`place()`). It works out `left = (stockWidth − width) / 2`, which is only right because the material starts at 0.
  - `fitOnMaterial()` (line 962) and `sheetLimit()` (line 952) size new work to the material, less `STOCK_MARGIN_MM` (10) or `SHEET_EDGE_MM` (5).
  - In `DesignStep.tsx`, the `Selected` panel has **"Centre it"** (line 213, the same corner sum again) and **"Fill the material"** (`fit(..., true)`, line 225). The second one uses `stockRect`'s centre, so it would follow a moved material.
  - All of these act on **one** thing (the artwork or a single item). Nothing centres the artwork and its added text together as a group, although `everythingRect()` (line 556) already gives the group's outline.
- **The off-the-material warning** ("Something reaches past the edge of the material") is in `DesignStep.tsx` (`overhangs`), `OperationsPanel.tsx` (line 66) and the assistant's `describeProject` (`assistantTools.ts` line 438). All three go through `stockRect`, so they follow a moved material for free.
- **The material is only used for previews and warnings.** `generateGcode` (`project.ts` line 1840) sends `offset: [rect.x, rect.y]` from `artworkRect`, and nothing about the material's position. Moving the material therefore can't change the G-code. Only moving the design can, and `setArtworkRect`, `setPosition` and `setItemRect` already clear an out-of-date program (`staleGcode()`). The preview `Simulation` (`Canvas.tsx` line 267) and `View3D` take `stock` as a rectangle, so they follow `stockRect` too.
- **Saved projects** store `snapshot(s)` of the `UNDOABLE` keys (line 611), which include `stockWidth` and `stockLength`.
- **Adjusting by hand already works in part.** You can drag on the canvas with `Handles.tsx`, and type "From left" / "From front" in `Selected`. There is **no** arrow-key nudge on the canvas.
- **No rotation anywhere.** Designs and the material are always square to the bed (the only `rotat` in the code is in `View3D.tsx`).
- **Pieces from idea 1** (not built yet; being built in parallel): `camera_still` kept in the engine on desktop and web, `kerf_core::camera::{Homography, CameraFit, bed_picture}`, a fit stored in canvas bed mm, and the `src/lib/camera.ts` wrappers (kept out of `ipc.ts`, so the assistant can't reach them). The thickness field is called `thickness` in `project.ts`, not `stockThickness` as idea 1's plan says.
- **Missing:**
  - a position for the material
  - centring the whole design as a group
  - material detection
  - a way to confirm and correct a detected outline
  - nudging with the keyboard.

**Other software:**
- LightBurn's camera is an overlay that you place things on by hand. Its alignment is a calibration step, not edge-finding.
- xTool's older workflow is crosshairs and dragging, with guide lines to help centre.
- Glowforge users report that placement from the camera is only right when the material's thickness is exact.

None of these traces the material's edge and centres the design on it automatically, so Kerf has no established behaviour to copy.

## Approach

### Part A: the material has a place (no camera needed)

1. **Add `stockLeft` and `stockFront` to the project state** (mm, default 0). They use the same convention as the artwork's `left` and `front`: from the bed's left edge, and from the edge on the origin side. That way an origin change treats the material exactly as it treats the design. `stockRect()` becomes:
   `x = stockLeft`, `y = origin === "backLeft" ? stockFront : bed.height − stockFront − stockLength`.
   Add both keys to `UNDOABLE`, so they are undone and saved. When an older saved project has no such keys, `restore`/`openProject` default them to 0.
2. **Make every "middle of the material" sum go through `stockRect`:** `centred()`, the `centre` button in `DesignStep.tsx`, and `fitOnMaterial`/`sheetLimit` (unchanged, because they only use sizes). Convert back from the rectangle with the existing `left`/`front` formulas (`setArtworkRect` shows how).
3. **"Centre everything on the material"**: a new store action, `centreOnMaterial()`. It moves the artwork **and every added item** by the same amount, so that the centre of `everythingRect` lands on the centre of `stockRect`. The size doesn't change. It goes in the Design step next to "Centre it", which still centres just the selected thing. It counts as one undo step.
4. **The material's place in the Material step** (`MaterialPanel.tsx`):
   - "From left" and "From front/back" fields
   - a "Put it back in the corner" button
   - corner text that reads "The material is 120 mm from the left and 40 mm from the front" rather than always "sits in the front-left corner".
   - Warn if the material reaches past the bed (extend the existing `tooBig` check to use the position).
5. **Moving the material carries the design with it** (see Open questions 1). When `stockLeft`/`stockFront` change through the fields or detection, the artwork and items shift by the same amount, so a layout made on the material stays on it. This goes through `setPosition`/`setItemRect`, which already clear old G-code.
6. **Nudge with the keyboard.** When the canvas has focus and something is selected, the arrow keys move it 1 mm, and 10 mm with Shift. This calls the same store actions as dragging. It only ever moves the design on screen; see Safety for why idea 3 must not put machine jogging on these keys in the same place.

### Part B: the camera finds the material (needs idea 1)

7. **Detection in Rust, in `kerf-core`** (new `crates/kerf-core/src/material.rs`, pure Rust, so the same code runs in wasm):
   `find_material(still: &RgbaImage, fit: &Homography, bed: Bed, seed: Option<(f64, f64)>) -> Result<FoundMaterial, String>`.
   1. Warp the still to the bed with idea 1's `bed_picture`, at **1 px/mm**. This is enough for a 1 mm answer, and small and fast in the browser worker. Pixels outside the camera's view are transparent and never count as material.
   2. Lightly blur it. For each pixel, work out its colour and its **texture**: the spread of brightness in a 5 × 5 mm window. Honeycomb, slats and clamps are busy; plywood, MDF, acrylic and card are plain.
   3. **Find a starting point (the seed).** If the person clicked on the material, start there. Otherwise start at the centre of the current material rectangle if that spot is plain, or else at the plainest spot on a 10 mm grid.
   4. **Grow a region** out from the seed, one neighbouring pixel at a time, as long as the colour stays close to the region's running average and the texture stays low. The limits come from the seed's own neighbourhood. Write it as an iterative queue, not recursion, because the wasm stack is small and a panic can't be recovered there.
   5. Tidy the region: fill holes (knots, old engraving, a pencil mark) and smooth the outline (close, then open, about 3 mm).
   6. **Find the outline:**
      - take the convex hull (monotone chain)
      - find the **smallest rectangle around it** (rotating calipers). This gives the material's centre, width, length and angle in bed mm.
      - The centre of that rectangle is also right for a round coaster.
   7. **Run the checks** and return them as plain-English `warnings`, so the outline is never silently trusted:
      - **Refuse** when the region is smaller than 20 × 20 mm, or covers more than 90% of what the camera sees: "Kerf couldn't tell the material from the bed. Click on the material to help."
      - The region touches the edge of the camera's view: "Part of the material is outside the picture, so that edge is a guess."
      - The region fills less than 90% of its rectangle: "It doesn't look like a rectangle. Check the outline." When it's close to a circle (about 78%), say "It looks round" instead.
      - The angle is more than 1°: "The material is at an angle of 4°."
      - The size differs from the typed width and length by more than 10 mm: show both.
      - The material is thicker than 3 mm and the camera height isn't known (see step 10): "Thick material looks shifted in the camera picture, so check the far edges."

   `FoundMaterial { corners: [(f64, f64); 4], centre, width, length, angle_deg, warnings: Vec<String> }` is all in canvas bed mm, `#[serde(rename_all = "camelCase")]`.

   *Alternative not chosen:* comparing against a picture of the **empty bed**. This is good on busy honeycomb, but it needs an extra picture kept up to date. It is thrown off by a change of light or the machine's own LED, and it fails on a router bed where clamps and old cuts change from job to job. It can be added later as an extra hint for the region-growing, if the scrap tests show single-picture detection isn't reliable enough.

8. **Command `find_material({ fit, bed, seed })`** returns `FoundMaterial` as JSON.
   - **Desktop:** `src-tauri/src/camera.rs` reads `AppState.camera_still` and calls `kerf_core::material::find_material`.
   - **Browser:** a `find_material` arm in `run()` (`crates/kerf-web/src/lib.rs`) reads `STATE.camera_still`. The reply is JSON, so it goes through the ordinary `core.ts` → `engine` path, with no byte route and no `needsDesktop`.
   - It refuses with a plain message when there is no still, or when the fit was made for a different bed size (the same rule as idea 1).
   - The wrapper `findMaterial()` goes in `src/lib/camera.ts`, **not** `ipc.ts`, so the assistant can't reach it.
9. **Confirm and adjust (the important step).** Nothing is applied until the person says so.
   - A **"Find the material"** button sits in the canvas camera control (from idea 1), with a copy in the Material step. It is greyed out, with the reason, until the camera has been lined up. If the latest still is marked "may be out of date" (idea 1, step 6), or there is none, it takes a new one first. That only opens the camera.
   - The canvas shows the camera picture with the **found outline** drawn on it (dashed, accent colour), and a small panel:
     - "Material found: 297 × 210 mm, at an angle of 0.4°"
     - the warnings
     - **Use this outline**, **Click on the material to try again**, and **Cancel**.
   - **Correct the outline:** four corner markers can be dragged (reuse `Handles.tsx`, as idea 1's lining-up screen does). The rectangle is worked out again from the four corners: the centre is their average, and the sides are the averages of opposite sides.
   - **Use this outline** sets `stockLeft`, `stockFront`, `stockWidth` and `stockLength` from it (see Open questions 2 for the size), and then runs `centreOnMaterial()`. Together these are **one undo step**, so a wrong detection is undone with a single Ctrl+Z.
     - **If the material is at an angle of more than 1°:** Kerf can't turn designs, so it keeps the largest square-to-the-bed rectangle that fits inside the outline, around the same centre. It says so: "The material is at an angle, so Kerf will use the straight part in the middle: 280 × 190 mm. Straighten the material for the full size."
     - The tilted outline stays drawn on the canvas as a guide for as long as the camera picture is shown. It is kept in `src/state/camera.ts` with the still's time, not saved in the project.
     - Turning the design to match the material is a separate idea, because it would change G-code.
   - **The size doesn't change.** If the design doesn't fit on the found material, the existing "reaches past the edge" warning appears, together with a **"Make it fit"** button that runs the existing "Fill the material" sizing for the whole group.
   - After that, the person adjusts with dragging, the fields or the arrow keys (Part A).
10. **Thick material (later, after the scrap test).** The fit is made at bed height. On a far side, the camera sees the material's **top** edge, which looks pushed outwards. On the near side it sees the bottom edge, which is roughly right. So the material looks too big, and its centre is off by about half the far-side shift: with the camera 400 mm up, 200 mm out and 18 mm stock, about 9 mm on that side and 4–5 mm at the centre. With an optional **"Camera height above the bed"** setting (in idea 1's Camera section), each outline edge facing away from the spot under the camera is pulled in by `distance × thickness ÷ height`. The spot under the camera is taken to be the bed point at the middle of the picture, which assumes the camera points roughly straight down. Until that setting exists, give the warning from step 7 instead.

**Later, needs idea 3:** a no-camera way to find the material is to jog the laser's red-dot pointer or the router bit to two opposite corners of the material and press "This is a corner" at each. That reads the machine position and moves the machine, so it belongs with idea 3 (control the machine) and the `gcode-safety-reviewer`.

## Changes, by area

- **Rust engine (`crates/...`):**
  - `kerf-core`: new `material.rs` with `find_material`, `FoundMaterial`, region-growing, convex hull, the smallest surrounding rectangle, the largest square-to-the-bed rectangle inside a tilted one, the checks, and tests. Use the `image` crate through `kerf_trace::image`, as idea 1 does. Write the hull and caliper code by hand (about 150 lines, easy to test) rather than adding `imageproc`, which would pull in `rayon` and more for wasm.
  - `kerf-web`: a `find_material` arm in `run()`.
  - No change to `kerf-gcode`, `kerf-grbl` or `kerf-make`.
- **Desktop backend (`src-tauri/...`):** `find_material` in `src-tauri/src/camera.rs` (the file idea 1 adds), registered in `generate_handler!`.
- **Browser stand-in (`src/web/...`):** nothing special. `find_material` falls through `core.ts` to the engine and comes back as JSON. Confirm with `web-parity-checker`. Nothing new for the README's "leaves out" list: placement works in the browser, though G-code there still needs the desktop app.
- **Interface (`src/components/...`, `src/state/...`):**
  - `project.ts`: `stockLeft`/`stockFront`, the new `stockRect`, `UNDOABLE`, `centred()`, `centreOnMaterial()`, a `setStockPlace()` that carries the design along, and defaults for older saved projects.
  - `MaterialPanel.tsx`: the position fields, "Put it back in the corner", the new corner text, and a "Find the material" copy.
  - `DesignStep.tsx`: "Centre it" goes through `stockRect`; add "Centre everything on the material" and "Make it fit".
  - `Canvas.tsx`:
    - draw the material at its new place
    - draw the found outline and its corner markers
    - add the Find button to the camera control
    - add arrow-key nudging.
  - `src/state/camera.ts`: `found` (the outline, the warnings, and the time of the still it came from), cleared when a new still is taken.
  - `src/lib/camera.ts`: the `findMaterial()` wrapper.
  - `assistantTools.ts`: `describeProject` adds the material's `leftMm`/`frontMm`, so the assistant's picture of the layout stays true. **No new tool:** the assistant may not run detection or read the camera. It can still change the material size as it does now. Whether it may also set the position: see Open questions 4.
- **Help text / README:**
  - In `Help.tsx`/`hints.ts`:
    - "Where the material sits" (Part A)
    - "Finding the material with the camera": what it looks for (plain material on a busy bed), clicking to help, correcting the corners, the angle rule, the thick-material shift, and "the outline is a guess: check it, then frame the job before cutting"
    - the arrow keys.
  - README "What it does": add to idea 1's camera bullet: "…and find the material, so the design can be centred on it."

## Safety

This doesn't touch the machine. There are no changes to `kerf-gcode`, `kerf-grbl`, `kerf-make`, `machine.rs` or the machine panel. Finding the material only opens the camera and reads a picture, and works with nothing connected. Applying an outline only changes where the design sits on screen, through the same actions as dragging. The program is cleared (`staleGcode`), so an old job can't be sent from the old position. The bed-fit check in `kerf-gcode` still refuses any job that goes off the bed. So `gcode-safety-reviewer` isn't required, **unless** the build touches any of those files.

**The real risk is a wrong outline**, which puts a correct job in the wrong place:
- On a **laser**, that means burning the honeycomb or slats. Resin and smoke build up there, so this is a fire risk, not just a damaged bed. It can also mean engraving past the edge of a sheet the person can't afford to lose.
- On a **router**, it is worse. Detection could include a **clamp** or a hold-down, or the found edge could be a few mm off on the clamped side. The bit then hits metal: a broken bit, thrown pieces, a damaged machine.

How the plan guards against this:
1. **Never applied without a click.** Taking a picture never moves the design. Detection only proposes an outline, drawn over the camera picture. "Use this outline" is a deliberate click, and one Ctrl+Z undoes it all.
2. **Doubt is shown, not hidden.** The checks in step 7 refuse unclear results and flag edges outside the picture, odd shapes, angles, thickness and size mismatches. Any warning makes "Use this outline" read "Use it anyway".
3. **Clamps.** Busy texture stops the region growing, so clamps are normally left out. But the region can still run along a clamp of similar colour. On a **router**, the confirm panel always adds: "Check the outline doesn't include a clamp. Keep the design at least 10 mm from any clamp." "Make it fit" keeps the existing 10 mm margin (`STOCK_MARGIN_MM`).
4. **Thick material**: the warning, then the correction in step 10.
5. **Frame first.** After applying, the Machine step's existing "Frame the job outline" (`MachinePanel.tsx`, laser off) is highlighted with: "Frame the job to check it sits on the material before you start."
6. **The work origin moved**: idea 1's warning on the camera layer applies to the outline too, because it is drawn in bed coordinates.
7. **The assistant can't reach it.** The wrapper stays out of `ipc.ts`, there is no new assistant tool, and still no route to `machine_action`.
8. **Arrow keys.** Nudging moves only the design. Idea 3 may add keyboard jogging, and it must never bind the same arrow keys where the canvas listens, or a nudge could become a machine move. Note this in idea 3's plan, and have the nudge handler ignore keys while the machine panel's jog controls have focus.

**Tests to add** (`cargo test -p kerf-core`, in `material.rs`):
- A synthetic picture of a plain rectangle (300 × 200 mm, square to the bed) on a honeycomb-like checker is found within 1 mm of its corners. The same rectangle turned 5° is found with the right angle and centre.
- With the seed outside the material, the result is refused or comes back as the clicked region, never a merge of the bed and the material.
- A plain grey "clamp" bar of a different colour touching the sheet is not included. One of the same colour is flagged by the rectangle-fill check.
- A rectangle running off the edge of the camera's view gets the "outside the picture" warning.
- A circle's centre is found within 1 mm, with the "looks round" warning.
- A picture that is all one colour is refused, not answered with the whole bed.
- The largest square-to-the-bed rectangle inside a tilted one stays inside it (check all four corners) and keeps its centre.
- No panic on an empty, 1 × 1 or fully transparent picture.

**TypeScript checks** (in the store, or a small test if the project gains one):
- with `stockLeft`/`stockFront` at 0, `stockRect` gives exactly what it gives today
- `centreOnMaterial` puts the centre of `everythingRect` on the centre of `stockRect` for both origins
- an older saved project opens with the material in the corner.

## Hardware unknowns

Only a real camera over a real bed can settle these:
1. Does texture-plus-colour region-growing tell material from **honeycomb, slats and an MDF router spoilboard**? The spoilboard is the hard case: plain, and a similar colour to plywood.
2. How well does it handle **shiny or clear material**? Clear acrylic shows the bed through it, and mirrored acrylic reflects the room. The expectation is that these fail and the person is asked to click or draw the outline. Masked acrylic should be fine.
3. **Light**: the machine's LED, a dark room, the shadow of the gantry across the sheet.
4. How far off the centre is for 3 mm and 18 mm stock, near the middle and near an edge, with and without the camera-height correction.

**Scrap test** (after idea 1's own scrap test):
1. Put a scrap sheet at an odd angle and spot away from the corner. Take a picture and press "Find the material". Note any warnings and correct the corners if needed.
2. Apply it, with a 20 mm square as the design.
3. **Frame** the job first (laser off), and see whether the frame sits in the middle of the sheet.
4. Only then mark it **at the lowest power**, with a hand on the stop.
5. Measure the gap from the square to each edge of the sheet. Left minus right and front minus back give the centring error.
6. Repeat with 18 mm stock, and on a router with the sheet clamped (frame only, no cutting, until the numbers are known).
7. Record the numbers in the help text and in this plan.

## Decisions

Decided 2026-10-24, going with the recommendations.

1. **When the material moves, does the design move with it?** *Decided: yes.* The layout was made on the material, so it should stay on it. Dragging the design itself is unaffected.
2. **When an outline is applied, does it replace the typed width and length?** *Decided:* the position always comes from the camera. The size comes from the camera too, unless it is within 10 mm of what was typed; then the typed size is kept, because a ruler beats a camera. The confirm panel shows both and has a "Keep my size" choice.
3. **Is a rectangle enough for v1?** Offcuts are often L-shaped or ragged. *Decided: yes,* using the smallest rectangle around the material, with the shape warning. Placing a design inside an irregular outline is a later idea.
4. **May the assistant set the material's position?** It can already set the size. *Decided: yes,* by adding `leftMm`/`frontMm` to the existing material tool, because it moves nothing but the drawing on screen. It still never runs detection or sees the camera.
5. **Should "Find the material" run on its own every time a picture is taken?** *Decided: no.* It should always be pressed, so that the outline is never applied by surprise.

## Build order

1. **The material's place (Part A, steps 1–5).** `stockLeft`/`stockFront`, the new `stockRect`, undo and saving, the Material step fields, design carried along, and "Centre everything on the material". Shippable on its own, with no camera. Check by hand in both versions: move the material, and the design, preview, 3D view and warnings follow; an older project opens unchanged.
2. **Arrow-key nudge (step 6).** Shippable on its own.
3. **`kerf-core::material`** with all its tests, on synthetic pictures. Nothing uses it yet. It needs idea 1's `camera.rs` (`bed_picture`, `Homography`) merged first.
4. **The `find_material` command** on desktop and web, plus the `findMaterial()` wrapper. Needs idea 1's `camera_still` and fit.
5. **Confirm and adjust (step 9)**: the outline on the canvas, the corner markers, the panel, "Use this outline" as one undo step, the tilted-material rule, "Make it fit", and the frame-first prompt.
6. **Help text and README.** Run the scrap test and record its numbers.
7. **Later:** the camera-height setting and thick-material correction (step 10), and the empty-bed comparison if the scrap tests call for it.

## Done when

- `pnpm build`, `pnpm build:web`, `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` all pass, including the new `kerf-core::material` tests.
- `web-parity-checker` confirms `find_material` has a browser path, and that the README's browser list is still accurate.
- `gcode-safety-reviewer` has been run if the build touched `kerf-gcode`, `kerf-grbl`, `kerf-make`, `machine.rs` or `MachinePanel.tsx` (for example, for the frame-first highlight).
- `assistantTools.ts` has no camera or detection tool, and `findMaterial` isn't in `ipc.ts`.
- By hand, on Windows desktop and in Chrome:
  - With no camera: type a material position, and the design moves with it. "Centre everything on the material" centres the artwork and its words together. Undo puts it back. Save and reopen keeps it. An older project opens with the material in the corner.
  - With a camera: a sheet at an odd spot is found. The outline can be dragged. "Use this outline" centres the design and one Ctrl+Z undoes it. A tilted sheet gives the "straight part in the middle" message. A plain bed with nothing on it is refused with the "click on the material" hint.
  - The arrow keys nudge the selected thing by 1 mm, and by 10 mm with Shift. They never send anything to the machine.
- The scrap test above has been run once, and its numbers recorded.
