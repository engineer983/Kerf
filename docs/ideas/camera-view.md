# Camera view of the bed

Status: planned. Reviewed 2026-10-24 by `web-parity-checker`, `release-checker` and `gcode-safety-reviewer`; their findings are folded in below.

## The idea

For anyone with a camera (usually a USB webcam) pointed at their laser or CNC bed. They press one button and get a still picture of what is on the bed right now, and they can see their design drawn on top of it. That shows them where the material really is before they start a job. At the moment Kerf only shows the material as a rectangle in the bed's origin corner.

One still is enough. There is no live video. Taking the picture never moves or starts the machine.

## What Kerf already has

- **No camera code at all.** Searching for `getUserMedia`, `mediaDevices`, `camera` and `webcam` finds only the Three.js view camera in `src/components/View3D.tsx` and the depth-estimation wording in `src/lib/depth.ts` and `src/lib/hints.ts`.
- **A bed drawn in millimetres:** `src/components/Canvas.tsx`. It is one SVG whose `viewBox` is bed millimetres, with Y running down from the back (`artworkRect`, `itemRect`, `stockRect` in `src/state/project.ts` all use this). The layers go: bed fill, grid, numbered ticks, the material picture (`materialUrl`, around line 454), the artwork, then the items. A rectified camera picture fits as one more `<image x=0 y=0 width=bed.width height=bed.height>` between the grid and the material picture.
- **Machine settings remembered between sessions:** `DEVICE_KEY = "kerf.device"` and `storedDevice()` in `src/state/project.ts`, kept in localStorage. Camera settings can follow the same pattern under their own key.
- **Commands that take raw image bytes:** `load_image`, `replace_image` and `set_depth_map` in `src-tauri/src/lib.rs` read `InvokeBody::Raw`. The browser shim (`src/web/core.ts`, the last lines of `invoke`) forwards a `Uint8Array` to the WASM engine as `file`. Images come back as `tauri::ipc::Response` on desktop (`src-tauri/src/gallery.rs`, `pictures.rs`) and through `picture(which)` in `crates/kerf-web/src/lib.rs`.
- **State the engine keeps:** `AppState` in `src-tauri/src/lib.rs` (image, design, gcode) and the `thread_local! STATE` in `crates/kerf-web/src/lib.rs`. The latest camera still can be kept there too, which is what idea 2 needs.
- **The `image` crate** (0.25; png, jpeg, webp, bmp, gif) is already in the workspace through `kerf-trace`, and `kerf-core` already uses it as `kerf_trace::image` (`lib.rs` line 14, plus `encode_png`).
- **The window's CSP** (`src-tauri/tauri.conf.json`) already allows `blob:` and `data:` images, and `media-src` falls back to `default-src 'self'`. A `MediaStream` assigned with `srcObject` isn't checked by CSP, so no change is needed. Use `srcObject`, never `<video src=blob:…>`. The browser build has no CSP.
- **Only bytes from `picture` come back to the browser as bytes.** `engine()` in `src/web/engine.ts` JSON-parses every reply from `run()`. Only the special `"picture"` branch in `src/web/worker.ts` (calling `picture_png(which)`) returns raw bytes, and it takes no arguments.
- **Capabilities** (`src-tauri/capabilities/default.json`): only `core:default`. Tauri has no camera permission to grant, because the webview handles the camera itself.
- **Missing:** opening a camera and taking a still, choosing a camera, lining the picture up with the bed, showing it on the canvas, and every platform permission setting.

## Approach

**How the camera is reached: through the webview's `navigator.mediaDevices.getUserMedia`, on desktop and in the browser alike.** The same TypeScript code then runs in both versions. WebView2 (Windows), WKWebView (macOS) and WebKitGTK (Linux) all support it, though each needs some platform setup (see Changes). The browser version is served over https (GitHub Pages) or from localhost, and both count as secure contexts, so it works there too.

*Alternative not chosen:* a Rust camera crate such as `nokhwa`. It would avoid webview quirks, but it brings native camera libraries on three platforms (Media Foundation, AVFoundation, V4L2) into an installer pipeline whose macOS and Linux builds have never been run. It also wouldn't help the browser version at all. A **network camera "picture address"** (an http URL that returns a JPEG, which many IP cameras and ESP32-cams offer) is a cheap later addition on desktop only, using `ureq`. It is left out of v1 (see Decisions).

Steps:

1. **Take a still (interface only).** Add `src/lib/camera.ts`:
   - First check `navigator.mediaDevices?.getUserMedia`. It is `undefined` on a non-secure page (for example `pnpm dev:web` reached over a LAN address) and may be missing on some desktop webviews (old macOS, a WebKitGTK built without media support). Say "This window can't use a camera (it needs a secure page, or a newer system webview)" instead of crashing.
   - `listCameras()`: `enumerateDevices()` filtered to `videoinput`. Labels are blank until permission has been given once, so ask for permission first, then list.
   - `takeStill(deviceId)`:
     - open the stream with `{ video: { deviceId, width: { ideal: 1920 }, height: { ideal: 1080 } } }`
     - wait for the first frame (`video.requestVideoFrameCallback`, or `loadeddata` plus a short settle, about 300 ms, so auto-exposure can catch up)
     - draw the frame to a canvas and `toBlob("image/jpeg", 0.9)`
     - **stop every track straight away**, so the camera light goes off.
   - Errors become plain messages:
     - "No camera was found."
     - `NotAllowedError` and `NotReadableError` both get the privacy hint, because Windows' "Let desktop apps access your camera" switch can give either: "Kerf couldn't use the camera. Check the camera privacy settings, or close any other program using it." On macOS add "System Settings > Privacy & Security > Camera". On Linux add "check that your user is in the `video` group".
   - The camera wrappers (`setCameraStill`, `cameraBedPicture`) live here in `camera.ts`, **not** in `ipc.ts`. `assistantTools.ts` imports all of `ipc`, so keeping them out puts them out of the assistant's reach by construction.
   - Send the JPEG as `new Uint8Array(await blob.arrayBuffer())`. The browser shim only forwards a `Uint8Array` or `ArrayBuffer` (`core.ts` lines 61-62); a `Blob` would be turned into `{}`.
2. **Keep the latest still in the engine.** A new command `set_camera_still` takes the JPEG as the raw body, with no other arguments, and stores it decoded, along with its pixel size, in `AppState.camera_still` (desktop) or `STATE.camera_still` (web). Anything else it needs (pixel size) comes from the decoded still. Idea 2 reads the picture from here.
   - **The still must survive unrelated work.** The web `load_image` arm does `*state = State { image: Some(loaded), ..State::default() }` (`crates/kerf-web/src/lib.rs` line 215), which would wipe it. Carry it over (`camera_still: state.camera_still.take()`) in `load_image`, `clear_image` and a new project, on desktop and web.
3. **Line the picture up with the bed (a four-point fit).** Add `crates/kerf-core/src/camera.rs`:
   - `struct CameraFit { points: [(Pixel, BedMm); 4] }` and `Homography::from_points(...) -> Result<Homography, String>`. This is a plain 8×8 solve. It refuses three points in a line and points too close together, with a plain-English error.
   - `Homography::to_bed(px, py) -> (x_mm, y_mm)` and `to_pixel(x_mm, y_mm)`. **This mapping is what idea 2 builds on.**
   - `bed_picture(still, &Homography, bed, px_per_mm) -> RgbaImage`. For each bed pixel it looks up the camera pixel through the inverse mapping (bilinear sampling), and anything outside the camera's view is transparent. Use 2 px/mm, capped so the longest side is at most 2400 px.
   - Bed millimetres use the canvas convention: X from the left, Y down from the back. Converting from what the person types (machine coordinates measured from the origin corner) uses `origin`, exactly as `itemRect` does.
   - **The fit is stored in canvas bed mm**, together with the bed size and origin it was made under. Changing the origin setting then leaves the picture exactly where it was. If the bed size changes, don't show the picture; ask for lining up again (as for a still of a different size).
   - The sampling code must not panic: wasm has no unwinding. Decoding a 1920×1080 JPEG and warping up to 2400×1800 pixels takes noticeable time in the browser; it runs in the worker, so that's fine.
4. **Command `camera_bed_picture({ fit, bed })`**: returns PNG bytes of the still, warped to the bed. Desktop returns it as `tauri::ipc::Response`. The browser needs a new byte route, because `run()` can only return JSON (see "What Kerf already has"):
   - a second wasm export in `crates/kerf-web`, `camera_picture(input_json) -> Vec<u8>`
   - a matching `command === "camera_bed_picture"` branch in `src/web/worker.ts` (and `EngineRequest`) that replies `{ bytes }`
   - `cameraBedPicture` in `src/web/engine.ts`, and a `handlers` entry in `src/web/core.ts` that returns an `ArrayBuffer` (otherwise `invoke`'s fall-through JSON-parses it).
   - Keep the arguments camelCase on the wire: `#[serde(rename_all = "camelCase")]` on `CameraFit` and the bed struct, with matching TypeScript types; `origin` is `"frontLeft"` or `"backLeft"`. (Tauri renames top-level arguments, but the web side deserializes the JSON as sent.)
   - *Not chosen:* returning base64 PNG inside JSON. It works with no new route, but is wasteful for a 2400 px picture.
5. **Lining-up screen ("Line up the camera").** This opens from the Device step's new **Camera** section:
   - Show the raw still large, with four draggable markers (reuse the approach in `Handles.tsx`).
   - Each marker defaults to one bed corner and shows the bed position it stands for. The person drags each marker onto that place in the picture. If a bed corner can't be seen, they can type a different position for that marker, for example the corner of a sheet pushed into the origin corner.
   - A "Check" view draws the bed grid over the raw still through the fit, so a bad fit is obvious. A symmetric grid hides a mirrored or rotated fit (easy with an upside-down camera), so also draw "0,0" with X and Y arrows at the origin corner, and each corner's name.
   - Warn if the four markers go round the picture in the opposite order from the bed corners: that means the picture is mirrored.
   - Save the fit in localStorage under `kerf.camera`, together with the camera's `deviceId` and the still's pixel size. If a later still comes back a different size, say the camera needs lining up again.
6. **Show it.**
   - **On the 2D canvas, under the design.** Add a `cameraUrl` layer after the grid and before `materialUrl`. A toggle, "Show the camera picture", and an opacity slider (default 70%) go in a small canvas corner control, next to a **"Take a picture of the bed"** button.
   - Before the camera has been lined up, the canvas shows nothing, and the button opens the lining-up screen instead.
   - Label the layer in the corner text: "Camera picture, taken 14:32".
   - **Old picture:** after a job runs, or the material size changes, dim the picture and add "may be out of date".
   - **Moved work origin:** the picture shows the bed corner as 0,0, but the job runs from the work origin, which "Zero XY here" (`MachinePanel.tsx` line 171) or framing can move. When connected and the reported work offset (`work_offset`, WCO, `kerf-grbl/src/connection.rs` line 492) isn't zero, or zero was set this session, show on the camera layer: "The work origin has been moved, so the picture doesn't show where the job will go." This only reads status the panel already has; it sends nothing.
   - **Thick material:** once the material is thicker than about 3 mm, add to the label that the picture is lined up at the bed surface, so the top of the material looks shifted.
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
- Height is not corrected either. The fit is made at the bed surface, so the top of thick material looks larger and shifted. The shift is roughly distance from under the camera × thickness ÷ camera height: with the camera 400 mm up, a point 200 mm out and 18 mm stock, about 9 mm. The help text gives this rule and example.
- A later version could add LightBurn-style lens calibration (a printed dot pattern) or a single "straighten the curve" slider (radial k1).

**Later: line up by pointing with the head, laser off.** The safest lining-up aid needs no laser power. Jog the head (the existing `$J=` jog, which can't fire the laser) to four typed machine positions, take a still at each, and click the cross-hair pointer (or the nozzle, which looks shifted because it sits above the bed). This records real machine positions, so it removes the mirrored-fit and origin-mismatch problems. It moves the machine, so it needs its own plan and the `gcode-safety-reviewer`. Firing a visible dot with the beam would need laser mode (`$32`) handled and hard caps on power and time, and is a separate plan again.

## Changes, by area

- **Rust engine (`crates/...`):**
  - `kerf-core`:
    - new `camera.rs` with `Homography`, `CameraFit`, `bed_picture`, and tests
    - use the `image` crate through `kerf_trace::image`, as `kerf-core` already does. Don't add it as a direct dependency: a second entry could pull in different features.
    - all of it stays in `kerf-core`; `src-tauri/src/camera.rs` only passes bytes in and out.
  - `kerf-web`:
    - `camera_still: Option<RgbaImage>` in `State`, kept across `load_image`, `clear_image` and a new project
    - a `set_camera_still` arm in `run()` (bytes in `file`; line 207)
    - a new export `camera_picture(input_json) -> Vec<u8>` for `camera_bed_picture` (see Approach step 4).
- **Desktop backend (`src-tauri/...`):**
  - `camera_still: Mutex<Option<CameraStill>>` in `AppState`, kept across loading or clearing a picture.
  - `set_camera_still(Request)` (raw body) and `camera_bed_picture(fit, bed) -> Response` in a new `src-tauri/src/camera.rs`, registered in `generate_handler!`.
  - **macOS:**
    - add `src-tauri/Info.plist` with only `NSCameraUsageDescription` = "Kerf uses the camera to take a picture of your machine's bed." (no microphone key; video only). Tauri 2 merges this file into the bundle, and embeds it in dev builds too (`embed_plist` is already in `Cargo.lock`). Check the prompt appears under `pnpm tauri dev`; in dev it's credited to the terminal, not Kerf.
    - The camera needs macOS 12 or later (WKWebView's capture permission), while the app's minimum is 10.13. Handle the failure with the "can't use a camera" message, and say "macOS 12 or later" in the help text.
    - **When signing is added later:** a signed, notarised build with the hardened runtime needs `com.apple.security.device.camera` in an `Entitlements.plist`, pointed to by `bundle.macOS.entitlements`, or the camera is silently refused. Not needed while builds are unsigned.
  - **Linux:** WebKitGTK has `enable-media-stream` off by default, and it silently denies camera requests unless the app handles them.
    - Add `[target.'cfg(target_os = "linux")'.dependencies] webkit2gtk = "2.0"` to `src-tauri/Cargo.toml`. It must resolve to the same 2.0.2 wry uses (check with `cargo tree`), or the types won't match. Check with `cargo tree -e features` whether `set_enable_media_stream` and `is_for_video_device` need a version feature. Use `webkit2gtk::glib` rather than adding `glib`.
    - `run()` has no `.setup()` yet. Add one that gets the main window and, in `with_webview`, under `cfg(target_os = "linux")`:
      - turns on `enable_media_stream` in the WebKit settings
      - connects `permission-request`: `allow()` only a `UserMediaPermissionRequest` that is for video and not audio, `deny()` everything else, and return `true` so the default handler doesn't also run. The signal carries no origin, so check `webview.uri()` is the bundled app instead.
    - In `tauri.conf.json`, set `bundle.linux.appimage.bundleMediaFramework: true` (otherwise the AppImage relies on the host's GStreamer) and `bundle.linux.deb.depends: ["gstreamer1.0-plugins-good"]` (for the camera element, `v4l2src`). Possibly `gstreamer1.0-pipewire` too; check on Ubuntu 22.04+.
    - Add GStreamer packages (`gstreamer1.0-plugins-base`, `-good`, `-bad`, `gstreamer1.0-libav`) to the apt line in `.github/workflows/release.yml` (line 35), so the AppImage can bundle them. `ci.yml` only needs them if CI starts building the AppImage.
  - **Windows:** WebView2 shows its own permission prompt and remembers the answer in its data folder (`%LOCALAPPDATA%\dev.kerf.app\EBWebView`). Check that the prompt appears, that `navigator.mediaDevices` exists at the app's `http://tauri.localhost` page, and that the privacy switch being off gives the plain message.
  - `capabilities/default.json`: no change. `getUserMedia` runs inside the webview and `with_webview` is Rust, so neither needs a Tauri permission.
- **Browser stand-in (`src/web/...`):**
  - `set_camera_still` goes through the existing fall-through in `core.ts` (bytes, then `engine`).
  - `camera_bed_picture` needs its own `handlers` entry in `core.ts`, plus the new byte route through `engine.ts` and `worker.ts` (Approach step 4).
  - There is no `needsDesktop`: the camera works in a browser. GitHub Pages serves over https with the default permissions policy, which allows the camera; anyone hosting the browser version themselves must not send `Permissions-Policy: camera=()`.
  - Confirm with `web-parity-checker`.
- **Interface (`src/components/...`, `src/state/...`):**
  - new `src/lib/camera.ts`: capture, plus the `setCameraStill` and `cameraBedPicture` wrappers (kept out of `ipc.ts`; `cameraBedPicture` returns an `ArrayBuffer` like `projectPicture`)
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
  - re-warp when `origin` changes (the picture stays put); hide it and ask for lining up again when the bed size changes.
  - **Do not add anything to `src/lib/assistantTools.ts`.** A camera still must never become `useProject.image` (no "use as artwork" button), because `pictureForModel` would then send it to an AI provider.
- **Help text / README:**
  - A "Camera" entry in `Help.tsx`/`hints.ts`: what it does, lining up, the limits (curved edges; the thick-material rule and example), the moved-origin warning, and "macOS 12 or later".
  - README "What has and has not been tested" and the release notes in `release.yml`: the camera is unproven on macOS and Linux until someone tries it. Mention the Linux GStreamer package for the deb.
  - A README "What it does" bullet: "See the bed. Take a picture with a camera over the bed and see the design on top of it."
  - Browser-version notes in the README: the browser asks before using the camera. Nothing new for the "leaves out" list, unless the network-camera address is added later. That would be desktop only, because browsers block such requests (CORS), and would need a line in the list.

## Safety

- **Doesn't touch the machine.** No change to `kerf-gcode`, `kerf-grbl`, `kerf-make`, `machine.rs` or the machine panel. Taking a still only opens the camera. It sends nothing to the serial port, and it works with no machine connected. So `gcode-safety-reviewer` isn't needed, unless the build ends up touching any of those files.
- **No marks burned for lining up.** LightBurn lines its camera up by having the laser burn targets. v1 uses points the person can already see on the bed, so lining up never runs the machine. If burned targets are wanted later, that becomes its own plan, with the full machine-safety rules and reviewer.
- **The assistant can't use it.** No assistant tool takes, reads or describes a camera still, and the wrappers live outside `ipc.ts`. The assistant also gets no new route to `machine_action`. The camera layer doesn't appear in `get_project` output, and a still never becomes the project picture.
- **Main risk: a believable picture in the wrong place.** The camera never changes job geometry, and the existing bed-fit check in `kerf-gcode` still applies. The safety review confirmed the canvas uses the same bed frame as the G-code for both origins. What can still mislead, most dangerous first, and how the plan handles it:
  1. a moved work origin: warning on the camera layer (Approach step 6)
  2. a fit that goes wrong when the origin or bed size changes: fit stored in canvas bed mm with its bed size and origin (step 3)
  3. a mirrored or rotated fit: origin arrows in the Check view and the winding warning (step 5)
  4. thick material: the shift rule in the label and help text
  5. an old picture: dimmed after a job or a material change.

  Help text and the layer label also say: "The camera picture is a guide. Check the position with the machine's frame before cutting."
- **Privacy.** The camera is opened only when the person presses the button, and closed straight after. The still is not saved to disk, put in projects, or sent anywhere.
- **Tests to add** (`cargo test -p kerf-core`):
  - an identity fit maps pixels to the same mm
  - a known square-to-trapezoid fit maps the four points exactly and the centre correctly
  - `to_bed(to_pixel(p)) ≈ p`
  - three points in a line, or two the same, are refused
  - `bed_picture` of a synthetic image with one coloured dot puts the dot at the expected bed mm (both `frontLeft` and `backLeft` origins)
  - areas outside the camera's view are transparent
  - changing the origin setting leaves the picture's pixels in the same place
  - a fit made for one bed size is refused for another
  - a fit whose markers go round in the opposite order is flagged as mirrored.

## Hardware unknowns

Only a real camera over a real bed can settle these:

1. **Each platform actually opens a camera inside Kerf's window.** The macOS and Linux installers have never been run at all.
   - **Windows:** the WebView2 prompt appears; the privacy switch being off gives the plain message.
   - **macOS** (Apple Silicon and Intel, from the DMG): the prompt names Kerf and shows the usage text; `navigator.mediaDevices` exists at `tauri://localhost`; after refusing, the message points at System Settings; it still works after replacing the app with a newer build (the ad-hoc signature changes, so macOS may ask again); the prompt appears under `pnpm tauri dev`.
   - **Linux** (AppImage and deb, on a clean Ubuntu 22.04 and a minimal desktop): `navigator.mediaDevices` exists (Ubuntu's WebKitGTK might be built without media support); a USB webcam works through the permission handler; the AppImage works with no GStreamer installed; the message when the user isn't in the `video` group or no camera is plugged in.
   - Everywhere: the camera light goes off after the still.
2. **Exposure and resolution.** Is the first frame after opening dark or blurred? Is the 300 ms settle enough? Does `ideal: 1920×1080` give the camera's best still?
3. **How far off an uncalibrated wide-angle lens is**, across the bed, compared with the corners.
4. **Scrap test:**
   1. Fix the camera.
   2. Line it up on the four bed corners.
   3. Put a scrap sheet at an odd angle somewhere in the middle and take a still.
   4. Draw a 20 mm square over the sheet's visible corner on the canvas.
   5. First trace the square with **Frame** (jog moves only, laser off) and compare it with the picture. Only then mark it **at the lowest power**, with a hand on the machine's stop.
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

1. **Where does the capture button live?** *Decided (confirmed after comparing it with the machine panel, the title bar and the Device step only):* on the canvas corner (seen in both the Design and Machine steps), with camera choice and lining up in the Device step. A copy in the machine panel can come later if people ask.
2. **Is a network camera address (http snapshot URL) needed in v1?** *Decided:* no. Add it later, desktop only, as a `fetch_camera_still(url)` command using `ureq`, `needsDesktop` in the browser, plus a README line.
3. **Keep the last still between sessions?** *Decided:* no, keep it in memory only. The bed will have changed by the next session, and an old picture shown as current is misleading.
4. **Save the still in projects?** *Decided:* no, for size and privacy.
5. **Correct lens curve in v1?** *Decided:* no, perspective only. Revisit after the scrap test numbers.

## Build order

1. `kerf-core::camera` (homography, `bed_picture`) and its tests. Shippable on its own; nothing uses it yet.
2. `src/lib/camera.ts` plus the Device step's Camera section: choose a camera, take a still, show the thumbnail. Check this by hand on Windows desktop and in Chrome.
3. Platform setup: Info.plist (macOS); the `webkit2gtk` dependency, `.setup()` hook and permission handler (Linux); the AppImage/deb GStreamer settings and the `release.yml` apt packages. Build it in CI. Run by hand where a machine is available; otherwise mark it unproven in the release notes.
4. `set_camera_still` and `camera_bed_picture` on desktop and web (including the new wasm byte route), plus the wrappers in `camera.ts`. The still survives loading or clearing a picture.
5. The lining-up screen, with the fit saved to `kerf.camera` in bed mm with its bed size and origin, the origin arrows and the mirrored warning.
6. The canvas layer, show/hide, opacity, and the "Take a picture of the bed" button. Re-warp when the origin changes; ask for lining up again when the bed size changes. The moved-origin warning, thick-material note and out-of-date dimming.
7. Help text, the README bullet and the "not tested" lines.

## Done when

- `pnpm build`, `pnpm build:web`, `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` all pass, including the new `kerf-core::camera` tests.
- `web-parity-checker` reports that both new commands have a browser path, and that the README's browser list is still accurate.
- `release-checker` has been run (the Info.plist, Linux setup, bundle config and `release.yml` all change).
- `src/lib/assistantTools.ts` is unchanged, the camera wrappers aren't in `ipc.ts`, and a camera still never becomes the project picture.
- `gcode-safety-reviewer` has been run if the build touched the machine panel (for example, to read the work offset for the warning).
- By hand, on Windows desktop and in Chrome:
  - pick a camera and take a still; the camera light goes off afterwards
  - line up on four bed corners; the grid "Check" view lines up with the real bed lines
  - the design shows over the picture in the right place
  - changing the origin keeps the picture in place; changing the bed size asks for lining up again
  - with the work origin moved, the warning shows on the camera layer
  - loading a new picture keeps the camera still
  - with camera access refused, no camera, or `pnpm dev:web` opened over a LAN address, a plain-English message appears and nothing breaks.
- The scrap test above has been run once, and its numbers recorded in the help text.
