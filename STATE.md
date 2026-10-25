# STATE.md — Current State & Task Tracker

> Last updated: 2026-10-25 05:55

## Current Phase
Phase 2.8: **Reasoning panel ergonomics + the New/Open deck actions.** Rust 146 tests and 28 UI tests green. The reasoning drawer now scrolls on its own, follows the tail while it streams, and expands to a large overlay on double click (Esc or the scrim collapses it). The topbar `Nuevo` button actually starts a new deck: it was posting the agent-only `create_presentation` command to `/api/command`, which rejected it with 422 and left the old slides untouched. `Abrir` no longer aborts halfway through with a `TypeError`.

Previous phase (2.7) — Copilot loop reliability & honest failure reporting: the system prompt is rebuilt from the live deck every round (a stale snapshot made the model re-create slides until the round cap), the cap is 20 rounds, three identical tool calls stop the turn, and the cap reports as `tool_limit` instead of a fake provider rejection.

## Implemented & Working

### Backend (Rust workspace, 5 crates, 142 tests green)
- `slyde-core`: `Presentation`/`Slide`/`Theme`/`AnimationStep` model, `PresentationState` with undo/redo, history limits, validation (`max_slides` 200, `max_code_bytes` 512 KiB, non-empty code, CSS-valid colors), and the `.slyde` project file (versioned JSON, validated on read). 29 tests.
- `slyde-server`: two listeners (app + sandbox origin), `/health`, `/api/state`, `/api/command`, `/api/agent`, `/api/export`, `/api/deck`, `/api/deck/open`, `/api/copilot/*`, `/print`, `/s/{id}`, `/fonts`, WS event stream, token + Origin guards. Chrome discovery/launch, runtime rendezvous file, agent command handling, structured `agent_activity` events, starter templates, atomic deck storage with debounced autosave. Added `DuplicateSlide` and `AddSlide` with explicit index target. Added `ChatEvent::Thinking` and reasoning content capture in `OpenAiAccum` and `AnthropicAccum`. 85 tests, including the copilot.
- **Copilot backend:** provider catalog for eight vendors, OpenAI-compatible transport for seven plus a native Anthropic Messages adapter, SSE assembly for fragmented tool calls, eight deck tools that call the shared `agent::run` path, typed error mapping, server-side key storage with env fallback, model normalization with free/vision/context metadata, and bounded image/PDF/DOCX/PPTX/text attachment extraction. `POST /api/copilot/chat` is a typed SSE stream with keep-alives; cancellation drops the upstream response.
- `slyde-export`: transport-agnostic Chrome pipeline. PDF via `--print-to-pdf` (960×540 pt = exact 16:9), PNG per slide at 1280×720. Unique Chrome profile per job, absolute paths, artifact verification, optional progress channel. 3 tests.
- `slyde-mcp`: hand-rolled JSON-RPC 2.0 stdio loop, 7 tools with hand-written schemas, thin HTTP proxy into the running studio, auto-spawn when no healthy runtime exists. `export_presentation` now uses the shared pipeline and deck-based default names. 10 tests.
- `slyde-app`: CLI (`app`, `mcp`, `export`, `version`, `help`), UI/assets dir resolution, runtime reuse, Chrome launch, welcome slide seeding. `export` CLI lives here so `slyde-export` stays free of transport dependencies. `SLYDE_DATA_DIR`, `SLYDE_UI_DIR`, `SLYDE_ASSETS_DIR` and `SLYDE_BROWSER` overrides. 2 tests.

### Frontend (`ui/`, vanilla ES modules, no bundler)
- **Topbar:** brand mark, inline deck title input with real-time saving/saved indicator, canvas zoom controls (`Fit`, `50%`, `75%`, `100%`, `125%`, `150%`), language switch (EN/ES), light/dark toggle, undo/redo, topbar quick export menu (PDF, PNG ZIP, .slyde), Present button.
- **Storyboard:** live thumbnails, HTML5 drag-and-drop reordering with spatial drop-line indicators, thumbnail hover action bar (duplicate, delete, move up, move down), layout template popover (Hero, Bullets, Diagram, Chart, Image, Code).
- **Stage:** centered canvas with responsive scaling + manual zoom controls, AI sweep generation animation, status bar with connection dot and chips for animations.
- **Inspector:** organized subtabs:
  - **Slide tab:** template badge, slide ID, quick duplicate & delete buttons, auto-saving speaker notes, animation step list with per-step play, collapsible developer mode.
  - **Deck tab:** visual theme swatches (Studio, Paper, Night) showing palette dots, aspect ratio, deck file save/open, and export section.
- **In-app Copilot:** contextual indicator pill (`Slide N: Template` vs `Entire deck`), quick prompt suggestion pills above composer, streaming SSE, thread with localized tool rows, provider sheet with 8 providers and model search.
- **Presenter & Speaker Console:**
  - Standard kiosk presenter: full-screen slide with auto-hiding HUD and progress bar.
  - **Speaker Console (`?speaker=1` or key `S`):** dual view with current slide, next slide preview, wall clock, stopwatch timer with play/pause/reset, speaker notes viewer with font size controls (`A-` / `A+`). All fully localized in EN & ES.
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
- Copilot provider path: `cargo test -p slyde-server` covers catalog endpoints, model normalization/free flags, key rejection before network, attachment extraction, tool schemas, OpenAI fragmented tool calls, Anthropic streaming (including parallel tool results in one user turn) and deck-summary context.
- Copilot browser E2E: provider config/key/model responses are mocked with CDP `Fetch.fulfillRequest`, so the suite proves the eight-provider sheet, key form focus, model search, Free filter, selection persistence, attachment chip/remove, and typed SSE text + tool rows + completion announcement without a real key or network.
- UI suite: 28 tests green (`node --test "tests/ui/*.test.mjs"`): 8 deterministic activity-state tests plus 20 browser E2E tests covering fonts, sandbox isolation, storyboard + arrow keys, EN/ES switch, theme toggle, notes autosave, `.slyde` export, real PDF/PNG downloads, agent activity, canvas selection + palette + structure tree, the animation timeline editor, KaTeX/Mermaid/highlighted code, presenter progress, in-studio presenter reveal, present menu → speaker console, the copilot panel/provider/model flow, copilot SSE streaming, the localized `tool_limit` bubble, `Nuevo` replacing the deck (and staying undoable), `Abrir` restoring deck and title from disk, the reasoning drawer (tail following, own scroll, double-click expand, Esc collapse) and the accessibility audit.
- Reasoning panel: `.thinking-stream` scrolls independently (`overscroll-behavior: contain`, focusable, `tabindex="0"`), pins to the tail only while the reader has not scrolled away, and expands into a `#thinking-layer` overlay appended to `<body>` on double click or via its own button (Esc or the scrim closes it, then the drawer returns to its message).
- Deck actions: `UiCommand::NewDeck` → `PresentationState::start_new_deck` (one starter slide, keeps the theme and ratio, one undo step), so `Nuevo` works and a stray confirmation is recoverable. `Abrir` blurs the title field before `applyView` so the file's title always wins.
- Copilot loop: the system prompt is rebuilt from `state.view()` on every round, so tools that change the deck are visible to the model immediately. Verified by unit test that the round cap surfaces as `tool_limit` (HTTP 200) instead of `upstream`, and by E2E that a `tool_limit` SSE event renders our own copy, hides the provider payload, and offers retry.

## In Progress
- Real-provider manual validation (needs user keys): OpenAI, Anthropic, Gemini, xAI, Groq, OpenRouter, DeepSeek and NVIDIA. Endpoints and wire formats are unit-tested/mocked; vendor-specific tool-calling quirks are not yet proven in this environment.

## Current Task
- [x] Plan execution: bimodal canvas, technical blocks, palette, teleprompter, structure tree
  - [x] **Bimodal canvas**: every block gets a persisted `data-id` (`slyde_core::ensure_element_ids` on write and on load), the studio stage runs an edit layer (`?edit=1`) with hover-free click selection, an in-canvas outline, a floating HUD (align, padding, AI, deselect), double-click inline editing committed through `mutate_element`, and the copilot pill shows `Target: #id`
  - [x] **Command palette** (`⌘K` / `Ctrl+K`): present, export, add slide, themes, ratios, undo/redo, language, and selection-scoped AI actions
  - [x] **Technical blocks**: vendored KaTeX 0.19.1, highlight.js 11.12.0 and Mermaid 11.12.0 on the sandbox origin; lazy loading, per-line code wrappers with numbers and `data-emphasis`, theme-aware syntax palette, export waits via `--virtual-time-budget=15000`
  - [x] **Teleprompter**: reading-time estimate from speaker notes (130 wpm) with on/behind/ahead pace in both speaker consoles
  - [x] **Structure tree**: the sandbox publishes its block list; the inspector lists it and selects by id
  - [x] Tests: canvas + palette + tree E2E, technical-blocks E2E, `html` id injector unit tests; Rust 142 green, UI 23 green
  - [x] Docs updated (`STATE.md`, `MEMORY.md`, `assets/vendor/README.md`)

## Not Implemented Yet (audited 2026-10-24)
- Drag & drop assets with an `assets/images` store and a `.slyde` bundle format for them
- Optimistic DOM updates / `startViewTransition` morphs; token-by-token visual fill
- `SlideElement.elements` is modelled and reachable through tools, but the canvas still edits one HTML blob per slide; nothing populates the structured array yet

## Known Issues / Bugs
- MSVC target cannot link on this machine (Windows SDK missing). Only `x86_64-pc-windows-gnu` is verified.
- PPTX export is intentionally not implemented (no silent stub).
- PNG export launches one headless Chrome per slide (~1–2 s each, sequential). Progress is visible, but a 50-slide deck takes over a minute; reuse a single Chrome via CDP if this becomes a real workflow.
- Slides can only reference `data:`/`blob:` images (sandbox CSP), so photo-rich decks are not possible yet.
- Autosave keeps a single working deck; there is no deck library or recent-files list. `Nuevo` is the only way to start over, and it replaces the working deck (undoable, but the file on disk is rewritten after 300 ms).
- Killing the server with `Stop-Process -Force` skips the Ctrl+C flush; the last debounced write is what survives.
- The activity chip reflects both MCP `/api/agent` calls and copilot chat (the copilot emits `thinking` on request start/between tool rounds). The copilot panel also has its own tab live dot and thread caret; the two signals are intentionally redundant.
- Copilot conversation history is not persisted across reloads. Keys and the last provider/model are stored; the thread lives in the page.
- Stop aborts the HTTP stream; there is no server-side cancel endpoint or partial-response persistence.
- The copilot is desktop-only: the right panel is hidden below 1100px, so the empty-state "Ask the copilot" CTA is hidden there too.
- Provider model lists include models that may not support tool calling or vision; the UI shows metadata when the provider supplies it but does not pre-validate capability. A tool-mentioning 4xx now maps to `unsupported` ("pick a model that supports tools") instead of a generic upstream error.
- The copilot stop conditions are a budget, not a guarantee: 20 rounds, plus an early exit after three identical tool calls. Very large decks still need a second "continue" turn.
- API keys are stored as plaintext in the app-data JSON with owner-only Unix permissions. There is no OS keychain integration.
- PDF/DOCX/PPTX extraction is text-only (no OCR, no layout or image extraction); scanned PDFs yield no text and are rejected with a readable error.
- Real provider calls are unverified with live keys; the test suite mocks the HTTP boundary. NVIDIA/Gemini tool-calling behavior in particular needs a real pass.
- UI coverage is end-to-end only. There is no component-level DOM runner (adding one would mean adopting a test dependency; the E2E suite covers the critical surfaces meanwhile).
- macOS and Linux are untested.

## Build & Platform Status
- Windows: **verified** with `cargo +stable-x86_64-pc-windows-gnu`. MSVC blocked by the missing Windows SDK (`LNK1181: kernel32.lib`).
- macOS: not verified (no machine).
- Linux: not verified (no machine).

## Backlog (Prioritized)
1. **Validate the copilot against live provider keys** (all eight), starting with OpenAI, Anthropic and OpenRouter; record model/tool quirks and add any provider-specific compatibility shims.2. **Persist copilot conversations** per deck (thread, tool traces, timestamps) and add token usage/cost estimates.
3. **PPTX export** behind an `Exporter` trait — research first (`ppt-rs` 0.2.37 unvalidated), validate real files in PowerPoint/Keynote/Google Slides.
4. **Image asset strategy** for `.slyde` decks (sandbox CSP currently allows only `data:`/`blob:`), including a bundle format decision.
5. **Faster PNG export**: one Chrome/CDP session for all slides, useful before decks grow beyond a handful.
6. **Deck library / recent files + new deck**, replacing the single-autosave-slot model.
7. **Speaker notes in presenter mode** (data exists and persists).
8. Component-level UI unit tests if a DOM runner is adopted; the E2E suite remains the contract test for core flows.

## Next Immediate Step
- The user should run the copilot with at least one real key and report any provider/model that fails; then either harden those adapters or move to PPTX research (explicit approval required before touching the export architecture).

## Completed
- [x] Reasoning drawer: tail-following stream with its own scroll, double-click expand to an overlay, Esc/scrim collapse, localized labels — 2026-10-25
- [x] `Nuevo` starts a real new deck (`UiCommand::NewDeck` + `start_new_deck`, undoable) and `Abrir` no longer crashes on `renderStage()` — 2026-10-25
- [x] Copilot loop reliability: per-round deck snapshot, 20 rounds, repeat guard, `tool_limit` error code, provider text kept out of the thread, stable reasoning drawer, localized thinking badges, re-render on language change — 2026-10-25
- [x] Manual animation editor in the inspector: add, edit kind/target/delay/duration, reorder and remove steps (`set_animation`, validated) — 2026-10-25
- [x] Presenter reveal steps advance one per click/arrow again (first step on slide entry); authoring contract now matches behaviour — 2026-10-25
- [x] Plan phases A–D: bimodal canvas, `⌘K` palette, KaTeX/hljs/Mermaid rendering, teleprompter pace, structure tree — 2026-10-25
- [x] Spec audit + fixes: `mutate_element` nested-markup corruption, `get_selected_element` payload, honest `search_asset`/`generate_image` contracts, truthful technical-slide prompt — 2026-10-24
- [x] Presenter keyboard: non-interactive slide frame, key forwarding, Escape from fullscreen — 2026-10-24
- [x] Presenter black-screen fix: hidden speaker console, lost animation steps, clipped present menu — 2026-10-24
- [x] In-app copilot: eight providers, streaming SSE, deck tools, attachments, tabbed panel, EN/ES, mocked E2E — 2026-10-24
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
