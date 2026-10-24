# Kerf

A desktop app (Windows, macOS, Linux) for laser engravers and CNC routers, which also runs in a browser. It turns a picture, drawing or text into G-code, with a preview first.

## Layout

One codebase builds every version:

- `src/` — the React 19 + TypeScript interface (Zustand state, Three.js 3D view, Tailwind 4). Shared by desktop and browser.
- `src-tauri/` — the Tauri 2 desktop backend. Commands are registered in `src-tauri/src/lib.rs` (`generate_handler!`).
- `src/web/` — the browser's stand-in for the desktop backend. `vite --mode web` aliases `@tauri-apps/api/core` and `/event` to `src/web/core.ts` and `event.ts`, which answer the same command names from WebAssembly or browser APIs.
- `crates/` — the Rust engine, used by both:
  - `kerf-core` — project and design logic, the same on desktop and web
  - `kerf-trace` — raster to SVG tracing
  - `kerf-gcode` — G-code generation (lasers, CNC outlines, relief, cut-outs with tabs)
  - `kerf-grbl` — GRBL/grblHAL serial connection and streaming (desktop only)
  - `kerf-make` — finger-jointed boxes and stacked layers
  - `kerf-ai` — assistant model providers (Claude, OpenAI, OpenRouter, Ollama)
  - `kerf-web` — the engine built as WebAssembly into `src/web/pkg` (generated, git-ignored)

## Commands

Package manager is pnpm.

- Desktop dev: `pnpm tauri dev`
- Browser dev: `pnpm dev:web` (builds the WASM first; port 1421)
- Rebuild WASM only: `pnpm wasm`
- Checks CI runs — run these before calling work done:
  - `pnpm build`
  - `pnpm build:web`
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`

The engine crates are built optimised even in dev (see `Cargo.toml`), so first builds are slow.

## Rules

- **Machine safety comes first.** No generated G-code or sender behaviour has been proven on real hardware. Any change to `kerf-gcode`, `kerf-grbl`, `kerf-make` or the machine panel must keep: the laser off (`S0`) on every rapid move, `M5` at the end of a job, jobs refused when they don't fit the bed, and correct units and origin. Add or update a test that pins the exact G-code lines. Use the `gcode-safety-reviewer` agent on these changes.
- **The assistant must never move or start a machine.** Don't give it tools that reach `kerf-grbl` or `machine_action`.
- **Every feature works in both versions, or says why not.** A new Tauri command needs a handler in `src/web/core.ts` — either a real one or `needsDesktop(...)` — and the README's "What the browser version leaves out" list must stay accurate. Use the `web-parity-checker` agent.
- **Platform differences** (paths, credential store, fonts, Inkscape, serial ports, bundling) belong in `src-tauri` and the release workflow. The macOS and Linux installers have never been run, so be careful there. Use the `release-checker` agent before tagging a release.
- **Writing style:** the README, help text and UI strings use plain, everyday English for hobbyists ("the laser is off while moving", not jargon). Keep that tone. Code comments explain why, briefly.

## Releases

Pushing a `v*` tag runs `.github/workflows/release.yml` (NSIS on Windows, DMG for both Mac architectures, AppImage + deb on Linux). A tag with a hyphen is a pre-release. The version lives in `package.json`, the root `Cargo.toml` and `src-tauri/tauri.conf.json` — keep all three in step.
