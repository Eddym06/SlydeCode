# STATE.md — Current State & Task Tracker

> Last updated: 2026-10-25 12:10

## Current Phase

Phase 2.9 — **Contract and security hardening after the repo audit.** Rust 161 tests
and 29 UI E2E tests green; `cargo fmt --check` and `cargo clippy -D warnings` are
clean and now enforced in CI (`.github/workflows/ci.yml`).

The audit found four defects worth fixing before anything else: a silent cross-slide
edit (`data-id` handles are unique per slide, not per deck), tool schemas that
published a subset of the animation kinds and themes the core accepts, an
unauthenticated `/s/{id}` on the sandbox origin, and an undo stack that filled with
one entry per keystroke while typing the title or the notes. All four are fixed and
covered by tests, including an E2E that drives a real canvas edit on the second
slide.

## Implemented & Working

### Backend (Rust workspace, 5 crates, 161 tests)

- `slyde-core`: `Presentation`/`Slide`/`Theme`/`AnimationStep` model, `PresentationState`
  with undo/redo, history limits, validation (`max_slides` 200, `max_code_bytes` 512 KiB,
  non-empty code, CSS-valid colors), and the `.slyde` project file (versioned JSON,
  validated on read). 50 tests.
- **Wire contract has one source of truth**: `slyde_core::ANIMATION_KIND_IDS` and
  `slyde_core::THEME_IDS` feed the MCP schema, the copilot schema and the copilot
  prompt. `AnimationKind::id()` is an exhaustive match, and both test suites assert
  the published schemas equal the core lists, so a new variant cannot be forgotten.
- **Element edits are scoped**: `mutate_element(element_id, slide_id, …)` refuses an
  unscoped handle that matches more than one slide (`CoreError::AmbiguousElement`).
  Generated `slyde-N` handles stay unique per slide document, so stored decks need no
  migration.
- `slyde-server`: two listeners (app + sandbox origin), `/health`, `/api/state`,
  `/api/command`, `/api/agent`, `/api/export`, `/api/deck`, `/api/deck/open`,
  `/api/copilot/*`, `/print`, `/s/{id}`, `/fonts`, WS event stream, token + Origin
  guards. Chrome discovery/launch, runtime rendezvous file, structured
  `agent_activity` events, starter templates, atomic deck storage with debounced
  autosave. 93 tests, including the copilot.
- **Two session secrets**: `token` authorizes `/api/*` and `/ws`; `sandbox_token`
  authorizes slide documents only and is what the editor embeds in every iframe URL.
  `/s/{id}` accepts either (exports and the CLI already hold the app token), and
  rejects everything else with 401. Tokens come from `getrandom` (256 bits).
- **Copilot backend**: provider catalog for eight vendors, OpenAI-compatible transport
  for seven plus a native Anthropic Messages adapter, SSE assembly for fragmented tool
  calls, deck tools that call the shared `agent::run` path, typed error mapping,
  server-side key storage with env fallback, model normalization, bounded attachment
  extraction. Network failures now carry the real cause chain (DNS/TLS/refused) instead
  of a generic "could not be reached".
- **Undo history coalesces typing**: consecutive commits with the same label and target
  within `Limits::coalesce_window_ms` (default 1200 ms) rewrite one entry, so a rename
  or a paragraph of speaker notes is one undo step. `coalesce_window_ms: 0` restores
  one-entry-per-commit for tests.
- `slyde-export`: transport-agnostic Chrome pipeline. PDF via `--print-to-pdf`
  (960×540 pt = exact 16:9), PNG per slide at 1280×720. Unique Chrome profile per job,
  absolute paths, artifact verification, optional progress channel. 3 tests.
- `slyde-mcp`: hand-rolled JSON-RPC 2.0 stdio loop, 11 tools with hand-written schemas,
  thin HTTP proxy into the running studio, auto-spawn when no healthy runtime exists.
  `upsert_slide` accepts `slide_id` without an `index`, and `mutate_element` takes an
  optional `slide_id`. 13 tests.
- `slyde-app`: CLI (`app`, `mcp`, `export`, `version`, `help`), UI/assets dir
  resolution, runtime reuse, Chrome launch, welcome slide seeding. `SLYDE_DATA_DIR`,
  `SLYDE_UI_DIR`, `SLYDE_ASSETS_DIR` and `SLYDE_BROWSER` overrides. 2 tests.

### Frontend (`ui/`, vanilla ES modules, no bundler)

- **Topbar:** brand mark, inline deck title input with real-time saving/saved indicator,
  canvas zoom controls (`Fit`, `50%`, `75%`, `100%`, `125%`, `150%`), language switch
  (EN/ES), light/dark toggle, undo/redo, quick export menu (PDF, PNG ZIP, .slyde),
  Present button.
- **Storyboard:** live thumbnails, HTML5 drag-and-drop reordering with spatial drop-line
  indicators, thumbnail hover action bar, layout template popover.
- **Stage:** centered canvas with responsive scaling + manual zoom, AI sweep generation
  animation, status bar with connection dot and chips for animations.
- **Bimodal canvas:** every block carries `data-id`, click to select, floating HUD
  (align, padding, AI, deselect), double-click inline editing committed through
  `mutate_element` **with the owning slide id**, structure tree in the inspector.
- **Inspector:** slide tab (template badge, id, duplicate/delete, speaker notes with
  caret-preserving autosave, animation timeline editor, developer mode) and deck tab
  (all six theme swatches, aspect ratio, deck file save/open, export).
- **In-app Copilot:** contextual indicator pill, suggestion pills, streaming SSE,
  localized tool rows, reasoning drawer with tail-following scroll and double-click
  expand, provider sheet with 8 providers and model search.
- **Presenter & Speaker Console:** kiosk presenter with auto-hiding HUD and progress bar;
  speaker console with next-slide preview, wall clock, stopwatch, scalable notes,
  reading-time pace. All localized in EN & ES.
- **Themes:** the inspector select, the theme swatches and the command palette all read
  one `THEME_CHOICES` list that mirrors `slyde_core::THEME_IDS` (studio, paper, night,
  terminal, academic, pitch).
- **Auth in the browser:** JSON calls send `Authorization: Bearer`; only the WS
  handshake, `<a download>` links and `window.open` still use the query form, because a
  browser cannot set headers there.
- **Accessibility:** AA contrast in light and dark, focus rings, roving tabindex in the
  storyboard, labels on every icon button, `aria-live` status, localized iframe labels,
  32px top-bar targets. EN and ES dictionaries have exact key parity (257 keys each).

### Persistence

- Every committed mutation (UI command, MCP call, undo/redo, file open) is autosaved,
  debounced 300 ms, to `%APPDATA%\SlydeCode\decks\current.slyde` (atomic temp+rename).
- Startup restores that deck; if the file exists but is invalid, autosave stays off for
  the session so the recoverable file is never overwritten.
- `Guardar copia` downloads the deck via `GET /api/deck`; `Abrir archivo…` uploads a
  `.slyde` through a file input to `POST /api/deck/open`, which validates before
  replacing the deck. Ctrl+C flushes the deck before shutdown.

### Verified end to end

- `cargo test --workspace`: 161 tests green (2026-10-25).
- `node --test "tests/ui/*.test.mjs"`: 29 tests green in headless Chrome over CDP,
  including the new cross-slide edit regression and the accessibility baseline.
- Live HTTP probe of the sandbox origin: `/s/{id}` → 401 without a token, 401 with a
  wrong token, 200 with the sandbox token and with the app token; `/print` embeds the
  sandbox token in its iframes and never the app token.
- Earlier phases verified export (PDF/PNG, correct page size and dimensions), fonts in
  sandbox and export, MCP stdio flows and `.slyde` save/open.

## In Progress

Nothing partially implemented. The next work is prioritised in the backlog below.

## Not Implemented Yet (audited 2026-10-25)

- Drag & drop assets with an `assets/images` store and a `.slyde` bundle for them.
- Optimistic DOM updates / `startViewTransition` morphs; token-by-token visual fill.
- `SlideElement.elements` is modelled and reachable through tools, but the canvas still
  edits one HTML blob per slide; nothing populates the structured array yet.
- PPTX export, deck library/recent files, component-level UI tests (no DOM runner).

## Known Issues / Bugs

- MSVC target cannot link on this machine (Windows SDK missing). Only
  `x86_64-pc-windows-gnu` is verified; the `rust-toolchain.toml` pins `stable`, so on
  Windows the target still has to be passed explicitly or configured in `.cargo/config`.
- PNG export launches one headless Chrome per slide (~1–2 s each, sequential). A 50-slide
  deck takes over a minute; reuse a single Chrome via CDP if this becomes a real workflow.
- Slides can only reference `data:`/`blob:` images (sandbox CSP), so photo-rich decks
  are not possible yet.
- Autosave keeps a single working deck; there is no deck library or recent-files list.
  `Nuevo` replaces the working deck (undoable, but the file on disk is rewritten after
  300 ms).
- Killing the server with `Stop-Process -Force` skips the Ctrl+C flush; the last
  debounced write is what survives.
- The inspector rebuilds itself with `innerHTML` on every state change. The notes caret
  and scroll are restored explicitly, but focus inside other inspector controls can
  still be lost while a remote change lands.
- Copilot conversation history is not persisted across reloads. Keys and the last
  provider/model are stored; the thread lives in the page.
- Stop aborts the HTTP stream; there is no server-side cancel endpoint or
  partial-response persistence.
- The copilot is desktop-only: the right panel is hidden below 1100px.
- Provider model lists include models that may not support tool calling or vision; the
  UI shows metadata when the provider supplies it but does not pre-validate capability.
- API keys are stored as plaintext in the app-data JSON with owner-only Unix
  permissions. There is no OS keychain integration (a Windows DACL is still missing).
- PDF/DOCX/PPTX extraction is text-only (no OCR, no layout or image extraction).
- Real provider calls are unverified with live keys; the suite mocks the HTTP boundary.
- UI coverage is end-to-end only; there is no component-level DOM runner.
- macOS and Linux are untested (the new CI job runs the Rust suite on Linux, which is
  not the same as a manual pass).

## Build & Platform Status

- Windows: **verified** with `cargo +stable-x86_64-pc-windows-gnu`. MSVC blocked by the
  missing Windows SDK (`LNK1181: kernel32.lib`); clippy and rustfmt must therefore be
  run through the gnu toolchain as well.
- macOS: not verified (no machine).
- Linux: not verified locally; the CI workflow runs fmt, clippy and `cargo test` on
  `ubuntu-latest`.

## Backlog (Prioritized)

1. **Validate the copilot against live provider keys** (all eight), starting with
   OpenAI, Anthropic and OpenRouter; record model/tool quirks and add shims where needed.
2. **Deck library / recent files + new deck**, replacing the single-autosave-slot model.
3. **Image asset strategy** for `.slyde` decks (sandbox CSP currently allows only
   `data:`/`blob:`), including a bundle format decision.
4. **Faster PNG export**: one Chrome/CDP session for all slides, at 2× for retina.
5. **Persist copilot conversations** per deck and add token/cost estimates.
6. **PPTX export** behind an `Exporter` trait — research first, validate in PowerPoint.
7. **Split `ui/static/app.js`** into the modules described in `ui/src/README.md`
   (state object + per-region render) and drop the `innerHTML` rebuild of the inspector.
8. Component-level UI unit tests if a DOM runner is adopted; E2E stays the contract.

## Next Immediate Step

- Run the copilot with at least one real key and report any provider/model that fails;
  then harden those adapters. Independently, pick the deck-library item: it is the
  biggest remaining product cliff.

## Completed

- [x] Audit follow-up: cross-slide element scoping, single-source wire contract, sandbox
      token + authenticated slides, coalescing undo history, notes caret, bearer auth,
      honest copilot network errors, fmt/clippy/CI gate, license and docs alignment —
      2026-10-25
- [x] Reasoning drawer: tail-following stream with its own scroll, double-click expand —
      2026-10-25
- [x] `Nuevo` starts a real new deck (`UiCommand::NewDeck` + `start_new_deck`, undoable) —
      2026-10-25
- [x] Copilot loop reliability: per-round deck snapshot, 20 rounds, repeat guard,
      `tool_limit` error code — 2026-10-25
- [x] Manual animation editor in the inspector — 2026-10-25
- [x] Presenter reveal steps advance one per click/arrow — 2026-10-25
- [x] Plan phases A–D: bimodal canvas, `⌘K` palette, KaTeX/hljs/Mermaid, teleprompter,
      structure tree — 2026-10-25
- [x] Spec audit + fixes: `mutate_element` nested-markup corruption, honest
      `search_asset`/`generate_image` contracts — 2026-10-24
- [x] In-app copilot: eight providers, streaming SSE, deck tools, attachments, EN/ES — 2026-10-24
- [x] Durable decks: `.slyde` JSON, autosave, download/upload — 2026-10-24
- [x] MCP end-to-end over stdio; PDF/PNG export verified — 2026-10-24
