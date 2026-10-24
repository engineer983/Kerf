# Kerf

An open-source desktop app for laser engravers and CNC routers. It takes a picture, a drawing or a few typed words, lays them out on a piece of material at real size, and writes the G-code to make them, with a preview of the finished piece first.

It runs on Windows, macOS and Linux, and most of it also runs in a web browser with nothing to install.

## Get it

The current builds are **release candidates for testing**.

- **Desktop app:** installers are on the [Releases](https://github.com/engineer983/Kerf/releases) page.
- **In a browser:** open <https://engineer983.github.io/Kerf/> in Chrome or Edge. Your pictures and projects stay on your computer; nothing is uploaded.

### What the browser version leaves out

It is the same interface and the same engine, so pictures, drawings, text, boxes, layers, the previews, G-code and the assistant all work. A web page is not allowed to do some things a program can:

- **It does not connect to a machine.** Export the G-code and send it with your machine's own software, or use the desktop app.
- **Fonts.** Chrome and Edge ask before letting the page use the fonts installed on your computer. If you say no, or use another browser, text is set in the Lato family that comes with Kerf.
- **Saved projects** are kept in the browser's own storage for the site. They are not shared with the desktop app, and clearing the browser's site data removes them.
- **API keys** for the assistant are kept in the browser's storage, which is less protected than the desktop app's use of the system's credential store.
- **Ollama** on your own computer only answers a web page it has been told about: start it with `OLLAMA_ORIGINS=https://engineer983.github.io`.
- **Not available:** searching the web for pictures, reading a product page in the toolbox, and opening PDF, AI and EPS drawings.

Other browsers can run it too, but save files to the downloads folder instead of asking where, and cannot use your installed fonts.

The installers are not signed, so the operating system warns before running them:

- **Windows:** choose "More info", then "Run anyway".
- **macOS:** right-click the app and choose Open the first time.

> **Before trusting it with a machine:** neither the generated G-code nor the built-in sender has been proven on real hardware yet. Make your first job on each machine a small one on scrap, at low power or as an air cut, with a hand near the machine's own stop. Reports of what happened are very welcome as [issues](https://github.com/engineer983/Kerf/issues).

## What it does

The work goes through six numbered steps: Device, Material, Design, Trace, Operations, Machine.

- **Design on the material.** Start from a picture (PNG, JPEG, WebP, BMP, GIF), an SVG drawing, text in any installed font, or a box; then add more text and drawings around it. Text can run straight, along a curve, or round the artwork. Everything is moved and sized on a canvas that shows the bed and the material in millimetres.
- **Remove a background.** A photograph's subject is found on your own computer and what is behind it taken away; correct it by drawing round what to keep or remove.
- **Lasers.** Engrave a photograph with its shading, fill shapes solidly, follow outlines, and cut the piece out in the same job.
- **CNC routers.** Follow outlines, carve a picture as a 3D relief, and cut the piece out with holding tabs. A photograph's shape is estimated with a depth model that runs on your own computer. Speeds are worked out from the bit, the material and the router's speed, from a toolbox of bits you can edit.
- **Without tracing.** A box maker draws finger-jointed panels from a box's size and the material's thickness, allowing for a laser's kerf or a router bit's width. Stacked layers slice a picture into pieces to cut from thin sheet and glue into a relief.
- **See the bed.** Take a picture with a camera over the bed, line it up once on four known places, and see the design on top of it. The browser asks before letting the page use the camera, and the picture never leaves your computer. On a Mac the camera needs macOS 12 or later.
- **Preview.** Watch the job play back, see roughly what the finished piece will look like, and turn it over in 3D.
- **Run it.** Export the G-code as a file, or connect to a GRBL or grblHAL machine to jog, home, set zero, frame, start, pause and stop.
- **Assistant.** An optional chat panel that can set a job up from a wish such as "make this as realistic as possible", using Claude, OpenAI, OpenRouter, or a model on your own computer through Ollama. It can also draw a picture, ask an image model for one, or search for pictures that are free to reuse. It can never move or start a machine.
- **Help.** A built-in guide to good practice at the laser and the router, with step-by-step instructions for common jobs.

Where [Inkscape](https://inkscape.org) is installed, Kerf runs it as a separate program to open PDF, AI and EPS drawings too.

## What has and has not been tested

Tested by automated tests and on-screen checks: tracing, G-code generation, the previews, boxes, layers, text, saved projects, and the assistant against real and stand-in model servers.

Not yet tested:

- Any G-code on a real laser or router.
- The GRBL sender against a real controller; it has only met a simulated one.
- Picture generation against a real image service.
- The macOS and Linux installers. They are built automatically and have not been run.
- The camera on macOS and Linux, and how closely a lined-up picture matches the bed on a real machine. On Linux the deb needs the `gstreamer1.0-plugins-good` package for the camera; the AppImage brings its own.
- The browser version in anything but Chrome on Windows, and its assistant against anything but a local Ollama model.

Not built yet: centerline tracing, calculated laser power and speed, an allowance for the bit's width when following outlines, a separate roughing pass for reliefs, and automatic tool changes.

## Layout

```
src/            React + TypeScript interface
  web/          what stands in for the desktop shell in a browser
src-tauri/      Tauri shell: the commands the interface calls
crates/
  kerf-trace/   picture → traced shapes
  kerf-gcode/   shapes and pictures → G-code
  kerf-make/    boxes and stacked layers
  kerf-core/    a project from picture to job, the same on the desktop and in a browser
  kerf-web/     kerf-core built as WebAssembly
  kerf-grbl/    serial connection and GRBL streaming
  kerf-ai/      model providers and tool calling
```

## Development

Requirements: [Node.js 22+](https://nodejs.org), [pnpm](https://pnpm.io), [Rust](https://rustup.rs) (stable), and the [Tauri 2 system prerequisites](https://tauri.app/start/prerequisites/) for your OS.

```sh
pnpm install
pnpm tauri dev          # run the desktop app
cargo test --workspace  # run the tests
pnpm tauri build        # build an installer for this computer
```

The browser version needs the WebAssembly target as well (`rustup target add wasm32-unknown-unknown`):

```sh
pnpm dev:web            # run it at http://localhost:1421
pnpm build:web          # build it into dist-web/
```

Pushing a tag such as `v1.0.0-rc.3` builds installers for all three systems and publishes them as a release; a tag with a hyphen in it is marked as a pre-release. Pushing to `main` publishes the browser version with GitHub Pages.

## Safety

Lasers and CNC routers can cause fires and serious injury. Never leave a running machine unattended, and wear eye protection rated for your laser's wavelength. This software comes with no warranty.

## Licence

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT licence](LICENSE-MIT), at your option.

The browser version includes the Lato fonts by Łukasz Dziedzic, under the [SIL Open Font License](src/web/fonts/OFL.txt).
