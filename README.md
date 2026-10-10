# Kerf

An open-source desktop app that takes an image (PNG, JPEG, WebP, BMP, GIF) to SVG, and from SVG to G-code, for laser engravers and CNC routers.

> **Status: early development.** Working today: image import, outline and colour tracing with live preview, SVG export, and G-code generation (laser line, fill and photo engraving; CNC profile, 3D relief carving and cutting the piece out with tabs), reliefs carved from an estimated depth map (Depth Anything V2 Small, run locally), a toolbox of router bits including tapered ball noses with calculated starting speeds, materials, moving, scaling and cropping the artwork on the canvas, a playback, finished-look and 3D preview, and G-code export. There is also a GRBL sender (connect, jog, home, zero, frame, start, pause, stop, console) that has only been tested against a simulated controller. There is an assistant panel that can read and change the project through Claude or any OpenAI-compatible server (hosted or local, such as Ollama); it has been tested against stand-in servers, not yet against a real provider. Not built yet: centerline tracing, calculated laser settings, streaming replies and MCP servers for the assistant. Neither the generated G-code nor the sender has been run on a real machine: start with low power or an air cut, and keep a hand on the machine's own stop.

## Goals

- **Image → SVG**: outline, colour-layer and centerline tracing with live preview.
- **G-code**: laser cut, fill and raster engraving; simple CNC profile cuts. Export to a file.
- **Sender**: connect to GRBL / grblHAL machines, jog, frame, run, pause and stop.
- **Assistant**: an optional chat panel using the model provider of your choice (hosted or local), with MCP support so tools such as an Inkscape MCP server can be plugged in. The assistant can never move the machine or start a job.
- A clean, simple interface.

## Layout

```
src/            React + TypeScript UI
src-tauri/      Tauri shell (commands, window)
crates/
  kerf-trace/   image → SVG paths
  kerf-gcode/   SVG / raster → G-code
  kerf-grbl/    serial connection and GRBL streaming
  kerf-ai/      model providers, tool calling, MCP client
```

## Development

Requirements: [Node.js 22+](https://nodejs.org), [pnpm](https://pnpm.io), [Rust](https://rustup.rs) (stable), and the [Tauri 2 system prerequisites](https://tauri.app/start/prerequisites/) for your OS.

```sh
pnpm install
pnpm tauri dev      # run the desktop app
pnpm dev            # UI only, in a browser at http://localhost:1420
cargo test --workspace
```

## Safety

Lasers and CNC routers can cause fires and serious injury. Never leave a running machine unattended, and wear eye protection rated for your laser's wavelength. This software comes with no warranty.

## Licence

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT licence](LICENSE-MIT), at your option.
