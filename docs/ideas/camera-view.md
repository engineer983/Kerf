# Camera view of the bed

Status: planned

## The idea

For anyone with a camera (usually a USB webcam) pointed at their laser or CNC bed. They press one button and get a still picture of what is on the bed right now, and they can see their design drawn on top of it. That shows them where the material really is before they start a job. At the moment Kerf only shows the material as a rectangle in the bed's origin corner.

One still is enough. There is no live video. Taking the picture never moves or starts the machine.

## What Kerf already has

- **No camera code at all.** Searching for `getUserMedia`, `mediaDevices`, `camera` and `webcam` finds only the Three.js view camera in `src/components/View3D.tsx` and the depth-estimation wording in `src/lib/depth.ts` and `src/lib/hints.ts`.
- **A bed drawn in millimetres:** `src/components/Canvas.tsx`. It is one SVG whose `viewBox` is bed millimetres, with Y running down from the back (`artworkRect`, `itemRect`, `stockRect` in `src/state/project.ts` all use this). The layers go: bed fill, grid, numbered ticks, the material picture (`materialUrl`, around line 454), the artwork, then the items. A rectified camera picture fits as one more `<image x=0 y=0 width=bed.width height=bed.height>` between the grid and the material picture.
- **Machine settings remembered between sessions:** `DEVICE_KEY = "kerf.device"` and `storedDevice()` in `src/state/project.ts`, kept in localStorage. Camera settings can follow the same pattern under their own key.
- **Commands that take raw image bytes:** `load_image`, `replace_image` and `set_depth_map` in `src-tauri/src/lib.rs` read `InvokeBody::Raw`. The browser shim (`src/web/core.ts`, the last lines of `invoke`) forwards a `Uint8Array` to the WASM engine as `file`. Images come back as `tauri::ipc::Response` on desktop (`src-tauri/src/gallery.rs`, `pictures.rs`) and through `picture(which)` in `crates/kerf-web/src/lib.rs`.
- **State the engine keeps:** `AppState` in `src-tauri/src/lib.rs` (image, design, gcode) and the `thread_local! STATE` in `crates/kerf-web/src/lib.rs`. The latest camera still can be kept there too, which is what idea 2 needs.
- **The `image` crate** (0.25, png/jpeg/webp) is already in the workspace through `kerf-trace`.
- **The window's CSP** (`src-tauri/tauri.conf.json`) already allows `blob:` and `data:` images. A `MediaStream` assigned with `srcObject` is not loaded from a URL, so `media-src` should not need changing. Check this.
- **Capabilities** (`src-tauri/capabilities/default.json`): only `core:default`. Tauri has no camera permission to grant, because the webview handles the camera itself.
- **Missing:** opening a camera and taking a still, choosing a camera, lining the picture up with the bed, showing it on the canvas, and every platform permission setting.

## Approach

**How the camera is reached: through the webview's `navigator.mediaDevices.getUserMedia`, on desktop and in the browser alike.** The same TypeScript code then runs in both versions. WebView2 (Windows), WKWebView (macOS) and WebKitGTK (Linux) all support it, though each needs some platform setup (see Changes). The browser version is served over https (GitHub Pages) or from localhost, and both count as secure contexts, so it works there too.

*Alternative not chosen:* a Rust camera crate such as `nokhwa`. It would avoid webview quirks, but it brings native camera libraries on three platforms (Media Foundation, AVFoundation, V4L2) into an installer pipeline whose macOS and Linux builds have never been run. It also wouldn't help the browser version at all. A **network camera "picture address"** (an http URL that returns a JPEG, which many IP cameras and ESP32-cams offer) is a cheap later addition on desktop only, using `ureq`. It is left out of v1 (see Decisions).

Steps:

1. **Take a still (interface only).** Add `src/lib/camera.ts`:
   - `listCameras()`: `enumerateDevices()` filtered to `videoinput`. Labels are blank until permission has been given once, so ask for permission first, then list.
   - `takeStill(deviceId)`:
     - open the stream with `{ video: { deviceId, width: { ideal: 1920 }, height: { ideal: 1080 } } }`
     - wait for the first frame (`video.requestVideoFrameCallback`, or `loadeddata` plus a short settle, about 300 ms, so auto-exposure can catch up)
     - draw the frame to a canvas and `toBlob("image/jpeg", 0.9)`
     - **stop every track straight away**, so the camera light goes off.
   - Errors become plain messages: "No camera was found", "Kerf was not allowed to use the camera — check the camera privacy settings", "The camera is being used by another program".
2. **Keep the latest still in the engine.** A new command `set_camera_still` takes the JPEG as the raw body and stores it decoded, along with its pixel size, in `AppState.camera_still` (desktop) or `STATE.camera_still` (web). Idea 2 reads the picture from here.
3. **Line the picture up with the bed (a four-point fit).** Add `crates/kerf-core/src/camera.rs`:
   - `struct CameraFit { points: [(Pixel, BedMm); 4] }` and `Homography::from_points(...) -> Result<Homography, String>`. This is a plain 8×8 solve. It refuses three points in a line and points too close together, with a plain-English error.
   - `Homography::to_bed(px, py) -> (x_mm, y_mm)` and `to_pixel(x_mm, y_mm)`. **This mapping is what idea 2 builds on.**
   - `bed_picture(still, &Homography, bed, px_per_mm) -> RgbaImage`. For each bed pixel it looks up the camera pixel through the inverse mapping (bilinear sampling), and anything outside the camera's view is transparent. Use 2 px/mm, capped so the longest side is at most 2400 px.
   - Bed millimetres use the canvas convention: X from the left, Y down from the back. Converting from what the person types (machine coordinates measured from the origin corner) uses `origin`, exactly as `itemRect` does.
4. **Command `camera_bed_picture({ fit, bed })`**: returns PNG bytes of the still, warped to the bed. Desktop returns it as `tauri::ipc::Response`. In the web version it is an engine command whose bytes come back the same way `picture` returns them.
5. **Lining-up screen ("Line up the camera").** This opens from the Device step's new **Camera** section:
   - Show the raw still large, with four draggable markers (reuse the approach in `Handles.tsx`).
   - Each marker defaults to one bed corner and shows the bed position it stands for. The person drags each marker onto that place in the picture. If a bed corner can't be seen, they can type a different position for that marker, for example the corner of a sheet pushed into the origin corner.
   - A "Check" view draws the bed grid over the raw still through the fit, so a bad fit is obvious.
   - Save the fit in localStorage under `kerf.camera`, together with the camera's `deviceId` and the still's pixel size. If a later still comes back a different size, say the camera needs lining up again.
6. **Show it.**
   - **On the 2D canvas, under the design.** Add a `cameraUrl` layer after the grid and before `materialUrl`. A toggle, "Show the camera picture", and an opacity slider (default 70%) go in a small canvas corner control, next to a **"Take a picture of the bed"** button.
   - Before the camera has been lined up, the canvas shows nothing, and the button opens the lining-up screen instead.
   - Label the layer in the corner text: "Camera picture, taken 14:32".
   - No 3D view in v1.
7. **Camera choice and settings** go in the Device step (`DevicePanel.tsx`), in a new **Camera** section:
   - a camera dropdown
   - a "Line up the camera" button
   - "Forget camera"
   - a thumbnail of the last still.

   The machine panel gets nothing new in v1 (see Decisions). The person asked for "a button to capture what is currently in the field of view of the bed", and the button lives on the canvas because both the Design and Machine steps show the canvas.

**Lens distortion and perspective in v1:**
- Perspective is corrected, because that's what the four-point fit does.
- Barrel (fisheye) distortion is **not** corrected. The help text says that pictures from wide-angle lenses curve near the edges, and that the corners are the most accurate places to line up.
- Height is not corrected either. The fit is made at the bed surface, so the top of thick material looks slightly larger and offset.
- A later version could add LightBurn-style lens calibration (a printed dot pattern) or a single "straighten the curve" slider (radial k1).

## Changes, by area

- **Rust engine (`crates/...`):**
  - `kerf-core`:
    - new `camera.rs` with `Homography`, `CameraFit`, `bed_picture`, and tests
    - add `image` as a direct dependency, the same version and features as `kerf-trace`.
  - `kerf-web`:
    - `camera_still: Option<RgbaImage>` in `State`
    - a `set_camera_still` arm in `call` (bytes in `file`)
    - a `camera_bed_picture` command that returns PNG bytes, the same way `picture` does.
- **Desktop backend (`src-tauri/...`):**
  - `camera_still: Mutex<Option<CameraStill>>` in `AppState`.
  - `set_camera_still(Request)` (raw body) and `camera_bed_picture(fit, bed) -> Response` in a new `src-tauri/src/camera.rs`, registered in `generate_handler!`.
  - **macOS:** add `src-tauri/Info.plist` with `NSCameraUsageDescription` = "Kerf uses the camera to take a picture of your machine's bed." Tauri 2 merges this file into the bundle. Check whether `pnpm tauri dev`, which has no bundle, needs `embed_plist` under `cfg(all(debug_assertions, target_os = "macos"))`.
  - **Linux:** WebKitGTK has `enable-media-stream` off by default, and it silently denies camera requests unless the app handles them. In `lib.rs` setup, use `window.with_webview(...)` under `cfg(target_os = "linux")` to:
    - turn on `enable_media_stream` in the WebKit settings
    - connect `permission-request`, allowing only `UserMediaPermissionRequest` for video (no audio) from the app's own origin
    - deny everything else.
  - **Windows:** WebView2 shows its own permission prompt. Check that this appears, and that Windows' "Let desktop apps access your camera" privacy switch gives a clear error when it is off.
  - `capabilities/default.json`: no change expected.
- **Browser stand-in (`src/web/...`):**
  - `set_camera_still` and `camera_bed_picture` go through the existing engine path in `core.ts` (bytes, then `engine`). There is no `needsDesktop`: the camera works in a browser.
  - Confirm with `web-parity-checker`.
- **Interface (`src/components/...`, `src/state/...`):**
  - new `src/lib/camera.ts` (capture) and `ipc.ts` wrappers (`setCameraStill`, `cameraBedPicture`)
  - new `src/state/camera.ts` (Zustand), holding:
    - `deviceId`
    - `fit`
    - `stillUrl`, `takenAt` and `bedPictureUrl`
    - `show` and `opacity`
    - `busy` and `error`.

    `fit` and `deviceId` are kept in localStorage `kerf.camera`. The still is kept in memory only.
  - new `CameraLineUp.tsx` (lining-up screen)
  - Camera section in `DevicePanel.tsx`
  - camera layer and corner control in `Canvas.tsx`
  - re-warp when `bed` or `origin` changes.
  - **Do not add anything to `src/lib/assistantTools.ts`.**
- **Help text / README:**
  - A "Camera" entry in `Help.tsx`/`hints.ts`: what it does, lining up, the limits (curved edges, thick material).
  - A README "What it does" bullet: "See the bed. Take a picture with a camera over the bed and see the design on top of it."
  - Browser-version notes in the README: the browser asks before using the camera. Nothing new for the "leaves out" list, unless the network-camera address is added later. That would be desktop only, because browsers block such requests (CORS), and would need a line in the list.

## Safety

- **Doesn't touch the machine.** No change to `kerf-gcode`, `kerf-grbl`, `kerf-make`, `machine.rs` or the machine panel. Taking a still only opens the camera. It sends nothing to the serial port, and it works with no machine connected. So `gcode-safety-reviewer` isn't needed, unless the build ends up touching any of those files.
- **No marks burned for lining up.** LightBurn lines its camera up by having the laser burn targets. v1 uses points the person can already see on the bed, so lining up never runs the machine. If burned targets are wanted later, that becomes its own plan, with the full machine-safety rules and reviewer.
- **The assistant can't use it.** No assistant tool takes, reads or describes a camera still. The assistant also gets no new route to `machine_action`. The camera layer doesn't appear in `get_project` output.
- **Indirect risk: misplacement.** A badly lined-up picture could lead someone to put the design off the material and onto the bed or honeycomb. The camera never changes job geometry, and the existing bed-fit check in `kerf-gcode` still applies. Help text and the layer label say: "The camera picture is a guide. Check the position with the machine's frame or a low-power outline before cutting."
- **Privacy.** The camera is opened only when the person presses the button, and closed straight after. The still is not saved to disk, put in projects, or sent anywhere.
- **Tests to add** (`cargo test -p kerf-core`):
  - an identity fit maps pixels to the same mm
  - a known square-to-trapezoid fit maps the four points exactly and the centre correctly
  - `to_bed(to_pixel(p)) ≈ p`
  - three points in a line, or two the same, are refused
  - `bed_picture` of a synthetic image with one coloured dot puts the dot at the expected bed mm (both `frontLeft` and `backLeft` origins)
  - areas outside the camera's view are transparent.

## Hardware unknowns

Only a real camera over a real bed can settle these:

1. **Each platform actually opens a camera inside Kerf's window.** Check the WebView2 prompt, WKWebView with the Info.plist in an unsigned, ad-hoc-signed DMG (macOS camera permission is tied to code signing, and may ask again after each update), and WebKitGTK with the permission handler, in both the AppImage and the deb. The macOS and Linux installers have never been run at all.
2. **Exposure and resolution.** Is the first frame after opening dark or blurred? Is the 300 ms settle enough? Does `ideal: 1920×1080` give the camera's best still?
3. **How far off an uncalibrated wide-angle lens is**, across the bed, compared with the corners.
4. **Scrap test:**
   1. Fix the camera.
   2. Line it up on the four bed corners.
   3. Put a scrap sheet at an odd angle somewhere in the middle and take a still.
   4. Draw a 20 mm square over the sheet's visible corner on the canvas.
   5. Run it as **an air cut or at the lowest power**, with a hand on the machine's stop.
   6. Measure how far the mark lands from where the picture showed it. Note the distance at the centre and near the edges, and with 3 mm and 18 mm thick stock.

   The numbers go in the help text and in idea 2's plan.

## Left in place for idea 2 (auto-centre on the material)

Idea 2 is planned separately (`docs/ideas/auto-centre.md`, not yet written). This plan leaves it:
- the latest still kept in the engine (`camera_still` in `AppState` and the web `STATE`), so detection can run in Rust on both versions
- `Homography::to_bed` and `to_pixel` in `kerf-core`, in the same bed-mm convention as `itemRect`
- `bed_picture`, giving a picture where pixels are a fixed px/mm from the bed's back-left corner. Detecting a rectangle in it turns straight into a bed rectangle.
- a note that the fit is at bed height. Idea 2 will need the material thickness (`stockThickness`, already in the project) if it corrects for height.

## Decisions

Decided 2026-10-24, going with the recommendations.

1. **Where does the capture button live?** *Decided:* on the canvas corner (seen in both the Design and Machine steps), with camera choice and lining up in the Device step. A copy in the machine panel can come later if people ask.
2. **Is a network camera address (http snapshot URL) needed in v1?** *Decided:* no. Add it later, desktop only, as a `fetch_camera_still(url)` command using `ureq`, `needsDesktop` in the browser, plus a README line.
3. **Keep the last still between sessions?** *Decided:* no, keep it in memory only. The bed will have changed by the next session, and an old picture shown as current is misleading.
4. **Save the still in projects?** *Decided:* no, for size and privacy.
5. **Correct lens curve in v1?** *Decided:* no, perspective only. Revisit after the scrap test numbers.

## Build order

1. `kerf-core::camera` (homography, `bed_picture`) and its tests. Shippable on its own; nothing uses it yet.
2. `src/lib/camera.ts` plus the Device step's Camera section: choose a camera, take a still, show the thumbnail. Check this by hand on Windows desktop and in Chrome.
3. Platform setup: Info.plist (macOS) and the WebKitGTK media-stream setting with its permission handler (Linux). Build it in CI. Run by hand where a machine is available; otherwise mark it unproven in the release notes.
4. `set_camera_still` and `camera_bed_picture` on desktop and web, plus the `ipc.ts` wrappers.
5. The lining-up screen, with the fit saved to `kerf.camera`.
6. The canvas layer, show/hide, opacity, and the "Take a picture of the bed" button. Re-warp when the bed or origin changes.
7. Help text and the README bullet.

## Done when

- `pnpm build`, `pnpm build:web`, `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` all pass, including the new `kerf-core::camera` tests.
- `web-parity-checker` reports that both new commands are answered in `src/web/core.ts`, and that the README's browser list is still accurate.
- If the release workflow or bundle config changed (Info.plist, Linux setup), `release-checker` has been run.
- `src/lib/assistantTools.ts` is unchanged, and the assistant has no route to the camera.
- By hand, on Windows desktop and in Chrome:
  - pick a camera and take a still; the camera light goes off afterwards
  - line up on four bed corners; the grid "Check" view lines up with the real bed lines
  - the design shows over the picture in the right place
  - changing the origin keeps the picture in place
  - with camera access refused or no camera, a plain-English message appears and nothing breaks.
- The scrap test above has been run once, and its numbers recorded in the help text.
