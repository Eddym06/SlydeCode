# STATE.md — Current State & Task Tracker

> Last updated: 2026-10-24 12:25

## Current Phase
Phase 2: **working vertical slice.** Rust workspace, local server, sandbox origin, Chrome launch, UI editor, presenter, PDF/PNG export and the MCP stdio server all run end to end. The next architectural step is PPTX export (needs research + approval).

## Implemented & Working

### Backend (Rust workspace, 5 crates, 64 tests green)
- `slyde-core`: `Presentation`/`Slide`/`Theme`/`AnimationStep` model, `PresentationState` with undo/redo, history limits, validation (`max_slides` 200, `max_code_bytes` 512 KiB, non-empty code, CSS-valid colors). 19 tests.
- `slyde-server`: two listeners (app + sandbox origin), `/health`, `/api/state`, `/api/command`, `/api/agent`, `/print`, `/s/{id}`, WS event stream, token + Origin guards. Chrome discovery/launch, runtime rendezvous file, agent command handling, starter templates. 32 tests.
- `slyde-export`: PDF via headless Chrome `--print-to-pdf` (960×540 pt = exact 16:9), PNG per slide at 1280×720. Absolute paths resolved before invoking Chrome. 3 tests.
- `slyde-mcp`: hand-rolled JSON-RPC 2.0 stdio loop, 7 tools with hand-written schemas, thin HTTP proxy into the running studio, auto-spawn when no healthy runtime exists. 10 tests.
- `slyde-app`: CLI (`app`, `mcp`, `export`, `version`, `help`), UI dir resolution, runtime reuse, Chrome launch, welcome slide seeding.

### Frontend (`ui/`, vanilla ES modules, no bundler)
- Editor: topbar, storyboard with live thumbnails, stage, inspector (theme, format, notes, animation list with per-step replay, delete), developer mode, EN/ES switch, dark/light toggle, undo/redo, status pill.
- Presenter: letterboxed 16:9 stage, animation playback on load, keyboard nav (←/→/space/PageUp/PageDown/Enter/Backspace), `F` fullscreen, `Esc` exit, auto-hiding HUD, progress bar.
- Sandbox documents self-scale (`ResizeObserver` + translate/scale), so thumbnails, stage, presenter and print all use the same document.

### Verified end to end
- `slyde.exe app` → Chrome opens the studio; `/health` and `/api/state` answer; slide iframes carry the strict CSP.
- MCP over stdio: `initialize` → `ping` → `tools/list` → `create_presentation` → 3× `upsert_slide` → `inspect_current_state` → `trigger_animation` failure path. A 3-slide deck was written entirely through MCP and rendered in the UI.
- `slyde.exe export --format pdf` → 1 page per slide, `/MediaBox [0 0 960 540]`; `--format png` → 1280×720 per slide.

## In Progress
- Nothing partially implemented. UI layout defects found in screenshots are fixed and re-verified at 1440×920.

## Current Task
- [x] Vertical slice: app + UI + sandbox + export + MCP
  - [x] Rust workspace and all five crates
  - [x] App server, sandbox origin, CSP, Chrome launch, rendezvous
  - [x] Editor and presenter UI with full EN/ES i18n
  - [x] PDF and PNG export
  - [x] MCP stdio server with 7 tools
  - [x] Tests written (64 passing)
  - [x] Build verified on Windows (GNU toolchain)
  - [x] Docs updated (this file + `MEMORY.md`)

## Known Issues / Bugs
- Geist Sans/Mono are not bundled: the UI falls back to Inter/system fonts. Bundling the woff2 files is pending (affects brand fidelity, not correctness).
- MSVC target cannot link on this machine (Windows SDK missing). Only `x86_64-pc-windows-gnu` is verified.
- `slydecode_studio_interactive_app.html` sits untracked in the repo root (a design mockup, 97 KB, created before the current UI). Decide whether to move it to `docs/reference/` or delete it.
- PPTX export is intentionally not implemented (no silent stub).
- `.slyde` project format (save/open) is not implemented; state lives in memory per session.

## Build & Platform Status
- Windows: **verified** with `cargo +stable-x86_64-pc-windows-gnu`. MSVC blocked by the missing Windows SDK (`LNK1181: kernel32.lib`).
- macOS: not verified (no machine).
- Linux: not verified (no machine).

## Backlog (Prioritized)
1. Bundle Geist woff2 into `assets/fonts/` and serve it from the app origin (design-system fidelity).
2. `.slyde` project format + open/save, so work survives a restart.
3. PPTX export behind an `Exporter` trait — research first (`ppt-rs` 0.2.37 unvalidated), validate in PowerPoint/Keynote/Google Slides.
4. Frontend component tests (storyboard, inspector, language switcher, generation overlay) + one e2e smoke flow.
5. Speaker notes surface in presenter mode (data already exists in the model).
6. Keyboard shortcuts and accessibility pass on the editor (focus order, labels, contrast audit).

## Next Immediate Step
- Decide with the user between (a) bundling Geist + accessibility pass, (b) `.slyde` persistence, or (c) PPTX research. Persistence is the highest user-visible gap: closing the app loses the deck.

## Completed
- [x] Vertical slice: core → server → sandbox → UI → export → MCP — 2026-10-24
- [x] MCP end-to-end over stdio, deck authored by an agent — 2026-10-24
- [x] PDF/PNG export verified (exact 16:9, absolute paths) — 2026-10-24
- [x] Presenter verified (animation playback, HUD, progress) — 2026-10-24
- [x] Fix thumbnail scaling and sandbox self-fit (ResizeObserver, 0×0 guard) — 2026-10-24
- [x] Create engineering charter and project memory files — 2026-10-24
- [x] Consolidate memory into `MEMORY.md` + `STATE.md` — 2026-10-24
- [x] Decide Chrome-only rendering (Wry, CEF, Ultralight, Servo evaluated) — 2026-10-24
- [x] Create the workspace folder structure per §6 — 2026-10-24
- [x] Write the design system spec and the reference mockup — 2026-10-24
