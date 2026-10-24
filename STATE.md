# STATE.md — Current State & Task Tracker

> Last updated: 2026-10-24 14:30

## Current Phase
Phase 2: **working vertical slice with durable decks, real brand type, in-app export, an AI activity signal and an automated UI suite.** The Rust workspace, local server, sandbox origin, Chrome launch, editor, presenter, PDF/PNG export (UI, CLI and MCP), MCP stdio server and `.slyde` persistence all run end to end. 96 Rust tests and 17 UI tests (8 state-machine + 9 browser E2E) are green. Next candidate work: the prompt-driven in-app copilot or PPTX export (needs research + approval).

## Implemented & Working

### Backend (Rust workspace, 5 crates, 96 tests green)
- `slyde-core`: `Presentation`/`Slide`/`Theme`/`AnimationStep` model, `PresentationState` with undo/redo, history limits, validation (`max_slides` 200, `max_code_bytes` 512 KiB, non-empty code, CSS-valid colors), and the `.slyde` project file (versioned JSON, validated on read). 29 tests.
- `slyde-server`: two listeners (app + sandbox origin), `/health`, `/api/state`, `/api/command`, `/api/agent`, `/api/export`, `/api/deck`, `/api/deck/open`, `/print`, `/s/{id}`, `/fonts`, WS event stream, token + Origin guards. Chrome discovery/launch, runtime rendezvous file, agent command handling, structured `agent_activity` events, starter templates, atomic deck storage with debounced autosave. 52 tests.
- `slyde-export`: transport-agnostic Chrome pipeline. PDF via `--print-to-pdf` (960×540 pt = exact 16:9), PNG per slide at 1280×720. Unique Chrome profile per job, absolute paths, artifact verification, optional progress channel. 3 tests.
- `slyde-mcp`: hand-rolled JSON-RPC 2.0 stdio loop, 7 tools with hand-written schemas, thin HTTP proxy into the running studio, auto-spawn when no healthy runtime exists. `export_presentation` now uses the shared pipeline and deck-based default names. 10 tests.
- `slyde-app`: CLI (`app`, `mcp`, `export`, `version`, `help`), UI/assets dir resolution, runtime reuse, Chrome launch, welcome slide seeding. `export` CLI lives here so `slyde-export` stays free of transport dependencies. `SLYDE_DATA_DIR`, `SLYDE_UI_DIR`, `SLYDE_ASSETS_DIR` and `SLYDE_BROWSER` overrides.

### Frontend (`ui/`, vanilla ES modules, no bundler)
- Editor: topbar, storyboard with live thumbnails, stage, inspector (theme, format, notes, animation list with per-step replay, deck file actions, **export**, delete), developer mode, EN/ES switch, dark/light toggle, undo/redo, status pill.
- **Export from the inspector:** `PDF` and `PNG images` buttons render through headless Chrome and download with the deck's name (`q3-review.pdf`, `q3-review.zip`). Buttons enter an `aria-busy` state, show per-slide progress from WebSocket events, disable during the job and surface localized, non-technical errors (no Chrome, empty deck, generic failure).
- **AI activity signal:** `/api/agent` broadcasts structured `agent_activity` events (active → done/error) with no free-form text. A 28px status chip shows localized writing/adjusting/animating/done/error states for at least 1s, keeps the done confirmation for 2.6s and feeds a polite live region. Writing a slide also sweeps a warm scanline over the stage when that slide is visible; after a successful `upsert_slide` the stage follows the written slide only if the user is idle and not focused in a form or the storyboard.
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
- In-app export: `POST /api/export?format=pdf` returned a valid PDF (`%PDF-`, correct page count); `format=png` returned a ZIP with one 1280×720 PNG per slide. Both carried the deck-based RFC 5987 filename; the staging directory under app-data was empty afterwards.
- MCP export over stdio (`export_presentation` → `format=pdf`) produced a valid PDF through the shared pipeline.
- AI activity: a real `/api/agent` call rendered the writing chip and announced it, followed the written slide when idle (the E2E started on another slide), and played the scanline on the followed slide; the chip then settled on `Slide ready` and the live region matched. The visual pass used a headless Chrome screenshot during the sweep.
- Persistence: edits autosaved (2 slides → file on disk), process restarted → deck restored intact; download produced a valid `.slyde`; upload replaced the deck (4:3, 1 slide) and persisted it; a corrupt upload returned 400 and left the deck untouched.
- Fonts: `document.fonts.check("16px Geist")` and `"16px Geist Mono"` are true in the editor **and** in a sandbox slide document; body computes to `Geist, Inter, …`.
- Accessibility: six text/token pairs per mode measured above 4.5:1; Tab produces a visible 2px focus ring; every button has an accessible name.
- UI suite: 17 tests green (`node --test "tests/ui/*.test.mjs"`): 8 deterministic activity-state tests plus 9 browser E2E tests covering fonts, sandbox isolation, storyboard + arrow keys, EN/ES switch, theme toggle, notes autosave, `.slyde` export, **real PDF/PNG downloads from the inspector (busy state, deck name, one PNG per slide in the ZIP)**, **agent activity + conditional auto-follow**, sandbox document, presenter progress and the accessibility audit.

## In Progress
- None. The AI activity signal increment is complete; the next increment needs a product decision.

## Current Task
- [x] Add the AI activity signal (overlay for MCP agents)
  - [x] Structured `agent_activity` WebSocket events emitted only by `/api/agent`
  - [x] Pure UI state machine with minimum active hold, done confirmation and reserved thinking state
  - [x] Status chip, canvas scanline and conditional auto-follow
  - [x] EN/ES strings, reduced-motion fallback and accessible live announcements
  - [x] Rust, state-machine and browser E2E tests
  - [x] Docs updated (`STATE.md`, `MEMORY.md`, `README.md`, design note)

## Known Issues / Bugs
- MSVC target cannot link on this machine (Windows SDK missing). Only `x86_64-pc-windows-gnu` is verified.
- PPTX export is intentionally not implemented (no silent stub).
- PNG export launches one headless Chrome per slide (~1–2 s each, sequential). Progress is visible, but a 50-slide deck takes over a minute; reuse a single Chrome via CDP if this becomes a real workflow.
- Slides can only reference `data:`/`blob:` images (sandbox CSP), so photo-rich decks are not possible yet.
- Autosave keeps a single working deck; there is no deck library, recent-files list or "new deck" action.
- The AI activity overlay is a signal layer, not a model: it reacts to MCP tool calls, so it cannot show "thinking" before the first call or token-by-token writing. A prompt-driven in-app copilot is not implemented.
- UI coverage is end-to-end only. There is no component-level DOM runner (adding one would mean adopting a test dependency; the E2E suite covers the critical surfaces meanwhile).
- macOS and Linux are untested.

## Build & Platform Status
- Windows: **verified** with `cargo +stable-x86_64-pc-windows-gnu`. MSVC blocked by the missing Windows SDK (`LNK1181: kernel32.lib`).
- macOS: not verified (no machine).
- Linux: not verified (no machine).

## Backlog (Prioritized)
1. **Prompt-driven in-app copilot**: the signal layer is done, but the app still has no model. A true "thinking" state needs an in-app provider or a streaming generation channel, plus a composer and cancel/retry.
2. **PPTX export** behind an `Exporter` trait — research first (`ppt-rs` 0.2.37 unvalidated), validate real files in PowerPoint/Keynote/Google Slides.
3. **Image asset strategy** for `.slyde` decks (sandbox CSP currently allows only `data:`/`blob:`), including a bundle format decision.
4. **Faster PNG export**: one Chrome/CDP session for all slides, useful before decks grow beyond a handful.
5. **Deck library / recent files + new deck**, replacing the single-autosave-slot model.
6. **Speaker notes in presenter mode** (data exists and persists).
7. Component-level UI unit tests if a DOM runner is adopted; the E2E suite remains the contract test for core flows.

## Next Immediate Step
- Pick the next increment with the user: the prompt-driven copilot (new provider/channel, needs an architecture proposal) or PPTX research (requires explicit approval before touching the export architecture).

## Completed
- [x] Add the AI activity signal for external MCP agents (structured WS events, chip, scanline, conditional follow, EN/ES, tests) — 2026-10-24
- [x] Integrate and harden PDF/PNG export (UI + API + MCP, ZIP, progress, E2E) — 2026-10-24
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
