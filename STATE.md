# STATE.md — Current State & Task Tracker

> Last updated: 2026-10-24 13:20

## Current Phase
Phase 2: **working vertical slice with durable decks, real brand type and an automated UI smoke suite.** The Rust workspace, local server, sandbox origin, Chrome launch, editor, presenter, PDF/PNG export, MCP stdio server and `.slyde` persistence all run end to end. 83 Rust tests and 7 browser E2E tests are green. Next candidate work: the AI generation overlay, then PPTX export (needs research + approval).

## Implemented & Working

### Backend (Rust workspace, 5 crates, 83 tests green)
- `slyde-core`: `Presentation`/`Slide`/`Theme`/`AnimationStep` model, `PresentationState` with undo/redo, history limits, validation (`max_slides` 200, `max_code_bytes` 512 KiB, non-empty code, CSS-valid colors), and the `.slyde` project file (versioned JSON, validated on read). 29 tests.
- `slyde-server`: two listeners (app + sandbox origin), `/health`, `/api/state`, `/api/command`, `/api/agent`, `/api/deck`, `/api/deck/open`, `/print`, `/s/{id}`, `/fonts`, WS event stream, token + Origin guards. Chrome discovery/launch, runtime rendezvous file, agent command handling, starter templates, atomic deck storage with debounced autosave. 41 tests.
- `slyde-export`: PDF via headless Chrome `--print-to-pdf` (960×540 pt = exact 16:9), PNG per slide at 1280×720. Absolute paths resolved before invoking Chrome. 3 tests.
- `slyde-mcp`: hand-rolled JSON-RPC 2.0 stdio loop, 7 tools with hand-written schemas, thin HTTP proxy into the running studio, auto-spawn when no healthy runtime exists. 10 tests.
- `slyde-app`: CLI (`app`, `mcp`, `export`, `version`, `help`), UI/assets dir resolution, runtime reuse, Chrome launch, welcome slide seeding. `SLYDE_DATA_DIR`, `SLYDE_UI_DIR`, `SLYDE_ASSETS_DIR` and `SLYDE_BROWSER` overrides.

### Frontend (`ui/`, vanilla ES modules, no bundler)
- Editor: topbar, storyboard with live thumbnails, stage, inspector (theme, format, notes, animation list with per-step replay, deck file actions, delete), developer mode, EN/ES switch, dark/light toggle, undo/redo, status pill.
- Presenter: letterboxed stage, animation playback on load, keyboard nav (←/→/space/PageUp/PageDown/Enter/Backspace), `F` fullscreen, `Esc` exit, auto-hiding HUD, progressbar with `aria-valuenow`.
- Sandbox documents self-scale (`ResizeObserver` + translate/scale), so thumbnails, stage, presenter and print all use the same document.
- **Bundled typography:** Geist Sans/Mono variable woff2 (1.8.2, SIL OFL 1.1) in `assets/fonts/`, served at `/fonts` on both origins, preloaded in the editor and presenter. PDF/PNG exports now embed Geist instead of falling back.
- **Accessibility pass:** AA contrast in light and dark (worst pair 4.7:1), focus ring preserved on inputs, roving tabindex in the storyboard with focus following selection, labels on every icon button, `aria-live` status, localized iframe labels, `aria-pressed` on the theme toggle, 32px top-bar targets.

### Persistence
- Every committed mutation (UI command, MCP call, undo/redo, file open) is autosaved, debounced 300 ms, to `%APPDATA%\SlydeCode\decks\current.slyde` (atomic temp+rename).
- Startup restores that deck; if the file exists but is invalid, autosave stays off for the session so the recoverable file is never overwritten.
- `Guardar copia` downloads the deck via `GET /api/deck` (`Content-Disposition`, RFC 5987 name); `Abrir archivo…` uploads a `.slyde` through a file input to `POST /api/deck/open`, which validates before replacing the deck.
- Ctrl+C flushes the deck before shutdown, so the debounce window cannot swallow the last edit.

### Verified end to end
- `slyde.exe app` → Chrome opens the studio; `/health` and `/api/state` answer; slide iframes carry the strict CSP.
- MCP over stdio: `initialize` → `ping` → `tools/list` → `create_presentation` → 3× `upsert_slide` → `inspect_current_state` → `trigger_animation` failure path. A 3-slide deck was written entirely through MCP and rendered in the UI.
- `slyde.exe export --format pdf` → 1 page per slide, `/MediaBox [0 0 960 540]`; `--format png` → 1280×720 per slide, rendered with Geist.
- Persistence: edits autosaved (2 slides → file on disk), process restarted → deck restored intact; download produced a valid `.slyde`; upload replaced the deck (4:3, 1 slide) and persisted it; a corrupt upload returned 400 and left the deck untouched.
- Fonts: `document.fonts.check("16px Geist")` and `"16px Geist Mono"` are true in the editor **and** in a sandbox slide document; body computes to `Geist, Inter, …`.
- Accessibility: six text/token pairs per mode measured above 4.5:1; Tab produces a visible 2px focus ring; every button has an accessible name.
- UI E2E suite: 7 tests green (`node --test "tests/ui/*.test.mjs"`) covering fonts, sandbox isolation, storyboard + arrow keys, EN/ES switch, theme toggle, notes autosave, `.slyde` export, sandbox document, presenter progress and the accessibility audit.

## In Progress
- Nothing partially implemented.

## Current Task
- [x] Brand type + accessibility + UI test coverage
  - [x] Bundle Geist variable woff2 with its OFL license and document the source
  - [x] Serve `/fonts` on the app and sandbox origins; allow `font-src 'self' data:` in the sandbox CSP
  - [x] Preload + `@font-face` in editor, presenter and slide documents
  - [x] Contrast audit and token fixes (`--ink-subtle` in both modes)
  - [x] Focus ring, labels, roles, tab order, hit targets, localized aria strings
  - [x] Dependency-free CDP test harness + 7 E2E tests
  - [x] Rust tests (83) and UI tests (7) green on Windows GNU
  - [x] Docs updated (`STATE.md`, `MEMORY.md`, `docs/design-system.md`, `assets/fonts/README.md`)

## Known Issues / Bugs
- MSVC target cannot link on this machine (Windows SDK missing). Only `x86_64-pc-windows-gnu` is verified.
- PPTX export is intentionally not implemented (no silent stub).
- Slides can only reference `data:`/`blob:` images (sandbox CSP), so photo-rich decks are not possible yet.
- Autosave keeps a single working deck; there is no deck library, recent-files list or "new deck" action.
- The AI generation overlay (streaming/loading/error states) does not exist yet: agent edits appear when they land, with no in-product progress surface.
- UI coverage is end-to-end only. There is no component-level DOM runner (adding one would mean adopting a test dependency; the E2E suite covers the critical surfaces meanwhile).
- macOS and Linux are untested.

## Build & Platform Status
- Windows: **verified** with `cargo +stable-x86_64-pc-windows-gnu`. MSVC blocked by the missing Windows SDK (`LNK1181: kernel32.lib`).
- macOS: not verified (no machine).
- Linux: not verified (no machine).

## Backlog (Prioritized)
1. **AI generation overlay**: streaming status, empty/loading/error states and the signature sweep, so a non-technical user sees the agent working instead of slides appearing from nowhere.
2. **PPTX export** behind an `Exporter` trait — research first (`ppt-rs` 0.2.37 unvalidated), validate real files in PowerPoint/Keynote/Google Slides.
3. **Image asset strategy** for `.slyde` decks (sandbox CSP currently allows only `data:`/`blob:`), including a bundle format decision.
4. **Deck library / recent files + new deck**, replacing the single-autosave-slot model.
5. **Speaker notes in presenter mode** (data exists and persists).
6. Component-level UI unit tests if a DOM runner is adopted; the E2E suite remains the contract test for core flows.

## Next Immediate Step
- Pick the next increment with the user: the AI generation overlay (recommended, closes the product's core promise visually) or PPTX research (requires explicit approval before touching the export architecture).

## Completed
- [x] Bundle Geist + accessibility pass + dependency-free UI E2E suite — 2026-10-24
- [x] Durable decks: `.slyde` JSON, autosave, download/upload — 2026-10-24
- [x] Vertical slice: core → server → sandbox → UI → export → MCP — 2026-10-24
- [x] MCP end-to-end over stdio, deck authored by an agent — 2026-10-24
- [x] PDF/PNG export verified (exact 16:9, absolute paths) — 2026-10-24
- [x] Presenter verified (animation playback, HUD, progress) — 2026-10-24
- [x] Fix thumbnail scaling and sandbox self-fit (ResizeObserver, 0×0 guard) — 2026-10-24
- [x] Create engineering charter and project memory files — 2026-10-24
- [x] Decide Chrome-only rendering (Wry, CEF, Ultralight, Servo evaluated) — 2026-10-24
- [x] Write the design system spec and the reference mockup — 2026-10-24
