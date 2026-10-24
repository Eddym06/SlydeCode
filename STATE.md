# STATE.md — Current State & Task Tracker

> Last updated: 2026-10-24 13:05

## Current Phase
Phase 2: **working vertical slice with durable decks.** Rust workspace, local server, sandbox origin, Chrome launch, UI editor, presenter, PDF/PNG export, MCP stdio server and `.slyde` persistence all run end to end. Next architectural step is PPTX export (needs research + approval).

## Implemented & Working

### Backend (Rust workspace, 5 crates, 64 tests green)
- `slyde-core`: `Presentation`/`Slide`/`Theme`/`AnimationStep` model, `PresentationState` with undo/redo, history limits, validation (`max_slides` 200, `max_code_bytes` 512 KiB, non-empty code, CSS-valid colors), and the `.slyde` project file (versioned JSON, validated on read). 29 tests.
- `slyde-server`: two listeners (app + sandbox origin), `/health`, `/api/state`, `/api/command`, `/api/agent`, `/api/deck`, `/api/deck/open`, `/print`, `/s/{id}`, WS event stream, token + Origin guards. Chrome discovery/launch, runtime rendezvous file, agent command handling, starter templates, atomic deck storage with debounced autosave. 41 tests.
- `slyde-export`: PDF via headless Chrome `--print-to-pdf` (960×540 pt = exact 16:9), PNG per slide at 1280×720. Absolute paths resolved before invoking Chrome. 3 tests.
- `slyde-mcp`: hand-rolled JSON-RPC 2.0 stdio loop, 7 tools with hand-written schemas, thin HTTP proxy into the running studio, auto-spawn when no healthy runtime exists. 10 tests.
- `slyde-app`: CLI (`app`, `mcp`, `export`, `version`, `help`), UI dir resolution, runtime reuse, Chrome launch, welcome slide seeding.

### Frontend (`ui/`, vanilla ES modules, no bundler)
- Editor: topbar, storyboard with live thumbnails, stage, inspector (theme, format, notes, animation list with per-step replay, deck file actions, delete), developer mode, EN/ES switch, dark/light toggle, undo/redo, status pill.
- Presenter: letterboxed 16:9 stage, animation playback on load, keyboard nav (←/→/space/PageUp/PageDown/Enter/Backspace), `F` fullscreen, `Esc` exit, auto-hiding HUD, progress bar.
- Sandbox documents self-scale (`ResizeObserver` + translate/scale), so thumbnails, stage, presenter and print all use the same document.

### Persistence
- Every committed mutation (UI command, MCP call, undo/redo, file open) is autosaved, debounced 300 ms, to `%APPDATA%\SlydeCode\decks\current.slyde` (atomic temp+rename).
- Startup restores that deck; if the file exists but is invalid, autosave stays off for the session so the recoverable file is never overwritten.
- `Guardar copia` downloads the deck via `GET /api/deck` (`Content-Disposition`, RFC 5987 name); `Abrir archivo…` uploads a `.slyde` through a file input to `POST /api/deck/open`, which validates before replacing the deck.
- Ctrl+C flushes the deck before shutdown, so the debounce window cannot swallow the last edit.

### Verified end to end
- `slyde.exe app` → Chrome opens the studio; `/health` and `/api/state` answer; slide iframes carry the strict CSP.
- MCP over stdio: `initialize` → `ping` → `tools/list` → `create_presentation` → 3× `upsert_slide` → `inspect_current_state` → `trigger_animation` failure path. A 3-slide deck was written entirely through MCP and rendered in the UI.
- `slyde.exe export --format pdf` → 1 page per slide, `/MediaBox [0 0 960 540]`; `--format png` → 1280×720 per slide.
- Persistence: edits autosaved (2 slides → file on disk), process restarted → deck restored intact; download produced a valid `.slyde`; upload replaced the deck (4:3, 1 slide) and persisted it; a corrupt upload returned 400 and left the deck untouched.
- 83 workspace tests green.

## In Progress
- Nothing partially implemented.

## Current Task
- [x] Durable decks (`.slyde`)
  - [x] Core project file: versioned JSON, validation, `PresentationState::replace`
  - [x] Atomic storage + debounced autosave + flush on exit
  - [x] `GET /api/deck` download and `POST /api/deck/open` upload
  - [x] Inspector file actions with EN/ES strings
  - [x] Tests written (core +9, server +9)
  - [x] Build verified on Windows (GNU toolchain)
  - [x] Docs updated

## Known Issues / Bugs
- Geist Sans/Mono are not bundled: the UI falls back to Inter/system fonts. Bundling the woff2 files is pending (affects brand fidelity, not correctness).
- MSVC target cannot link on this machine (Windows SDK missing). Only `x86_64-pc-windows-gnu` is verified.
- PPTX export is intentionally not implemented (no silent stub).
- Slides can only reference `data:`/`blob:` images (sandbox CSP), so photo-rich decks are not possible yet.
- Autosave keeps a single working deck; there is no deck library or recent-files list.
- No frontend automated tests yet; UI behaviour is verified manually through screenshots and CDP measurements.

## Build & Platform Status
- Windows: **verified** with `cargo +stable-x86_64-pc-windows-gnu`. MSVC blocked by the missing Windows SDK (`LNK1181: kernel32.lib`).
- macOS: not verified (no machine).
- Linux: not verified (no machine).

## Backlog (Prioritized)
1. Bundle Geist woff2 into `assets/fonts/` and serve it from the app origin (design-system fidelity).
2. Frontend component tests (storyboard, inspector, language switcher, generation overlay) + one e2e smoke flow.
3. PPTX export behind an `Exporter` trait — research first (`ppt-rs` 0.2.37 unvalidated), validate in PowerPoint/Keynote/Google Slides.
4. Speaker notes surface in presenter mode (data already exists and now persists).
5. Keyboard shortcuts and accessibility pass on the editor (focus order, labels, contrast audit).
6. Image assets for slides: the sandbox CSP allows `data:`/`blob:` only, so a `.slyde` deck with photos needs an asset strategy before the format can hold binaries.

## Next Immediate Step
- Decide with the user between (a) bundling Geist + accessibility pass, (b) frontend tests, or (c) PPTX research. The deck now survives restarts, so the next gap is brand fidelity and test coverage of the UI.

## Completed
- [x] Durable decks: `.slyde` JSON, autosave, download/upload — 2026-10-24
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
