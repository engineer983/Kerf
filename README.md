# Kerf

An open-source desktop app for laser engravers and CNC routers. It takes a picture, a drawing or a few typed words, lays them out on a piece of material at real size, and writes the G-code to make them, with a preview of the finished piece first.

It runs on Windows, macOS and Linux.

## Download

Installers are on the [Releases](https://github.com/engineer983/Kerf/releases) page. The current builds are **release candidates for testing**.

The installers are not signed, so the operating system warns before running them:

- **Windows:** choose "More info", then "Run anyway".
- **macOS:** right-click the app and choose Open the first time.

> **Before trusting it with a machine:** neither the generated G-code nor the built-in sender has been proven on real hardware yet. Make your first job on each machine a small one on scrap, at low power or as an air cut, with a hand near the machine's own stop. Reports of what happened are very welcome as [issues](https://github.com/engineer983/Kerf/issues).

## What it does

The work goes through six numbered steps: Device, Material, Design, Trace, Operations, Machine.

- **Design on the material.** Start from a picture (PNG, JPEG, WebP, BMP, GIF), an SVG drawing, text in any installed font, or a box; then add more text and drawings around it. Text can run straight, along a curve, or round the artwork. Everything is moved and sized on a canvas that shows the bed and the material in millimetres.
- **Lasers.** Engrave a photograph with its shading, fill shapes solidly, follow outlines, and cut the piece out in the same job.
- **CNC routers.** Follow outlines, carve a picture as a 3D relief, and cut the piece out with holding tabs. A photograph's shape is estimated with a depth model that runs on your own computer. Speeds are worked out from the bit, the material and the router's speed, from a toolbox of bits you can edit.
- **Without tracing.** A box maker draws finger-jointed panels from a box's size and the material's thickness, allowing for a laser's kerf or a router bit's width. Stacked layers slice a picture into pieces to cut from thin sheet and glue into a relief.
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

Not built yet: centerline tracing, calculated laser power and speed, an allowance for the bit's width when following outlines, a separate roughing pass for reliefs, and automatic tool changes.

## Layout

```
src/            React + TypeScript interface
src-tauri/      Tauri shell: the commands the interface calls
crates/
  kerf-trace/   picture → traced shapes
  kerf-gcode/   shapes and pictures → G-code
  kerf-make/    boxes and stacked layers
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

Pushing a tag such as `v1.0.0-rc.2` builds installers for all three systems and publishes them as a release; a tag with a hyphen in it is marked as a pre-release.

## Safety

Lasers and CNC routers can cause fires and serious injury. Never leave a running machine unattended, and wear eye protection rated for your laser's wavelength. This software comes with no warranty.

## Licence

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT licence](LICENSE-MIT), at your option.
