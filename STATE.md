# STATE.md — Current State & Task Tracker

> Last updated: 2026-10-25 14:45

## Current Phase

Phase 3.0 — **product completion on `feat/product-completion`.** Rust 217 tests and
41 UI tests green; `cargo fmt --check` and `cargo clippy -D warnings` clean.

This block closed the three product gaps the audit named — a real deck library,
real images, and PowerPoint export — plus the infrastructure that had been
missing: batch PNG rendering over CDP at 2x, a concurrent MCP stdio loop, a
server-side copilot cancel, per-locale dictionaries with parity tests, and a
module smoke test that would have caught the dead `app.js` import.

Two data bugs were found and fixed along the way, both invisible to the old
suite:

- the autosave wrote into a placeholder `unset.slyde` instead of the active deck
  (`watch::Sender::send` silently drops the value when nobody is subscribed yet),
  so edits looked saved and were not;
- the UI suite was driving the previous build, because `findBinary()` preferred
  the gnu profile by list order instead of by recency.

## Implemented & Working

### Deck library (new)

- `<app-data>/decks/<id>.slyde`, one file per deck, plus `settings.json` inside
  the same directory for the active pointer. The directory is the source of
  truth: no index to drift, and a corrupt file is skipped instead of hiding the
  rest of the library.
- Endpoints: `GET /api/decks`, `POST /api/decks/{new,open,rename,duplicate,delete}`.
  Every operation flushes the outgoing deck first, then swaps the pointer, the
  in-memory state and the published view.
- Deck ids are ASCII slugs derived from the title (`Presentación sin título` →
  `presentacion-sin-titulo-1a2b`) and validated against traversal on every lookup.
- Startup imports the pre-library `current.slyde` once, and opens a healthy deck
  when the active file is unreadable (leaving the broken file untouched).
- UI: a Library sheet (search, relative timestamps, inline rename, duplicate,
  delete with an in-sheet confirmation), a topbar button, and `⌘K` entries.
  "New" now creates a deck file instead of replacing the open one.

### Images (new)

- Asset store at `<app-data>/assets/`, random ids (`<16 hex>.<png|jpg|webp|gif>`),
  served at `/assets/<name>` on **both** origins so slides can reference them
  under the sandbox CSP. SVG is refused: an SVG is a document, not a picture.
- `POST /api/assets` accepts a base64 data URL (what a paste produces) and
  answers with the stored URL; uploads are capped at 12 MiB and the format is
  decided by the bytes, not by the declared mime type.
- Canvas: paste (Ctrl+V), drag-and-drop onto the stage, an inspector **Image**
  button, drag to move, Shift+drag to resize, arrow-key nudge (1px / 10px with
  Shift). Geometry is committed as stage pixels through `set_geometry`, which
  **replaces** the declarations instead of appending them, so dragging does not
  accumulate stale `left:` values.
- `.slyde` v2: a deck that references stored images saves as a ZIP bundle
  (`deck.json` + `assets/…`); a deck without images stays plain JSON. The reader
  sniffs the ZIP signature, so both shapes open with the same button.

### PowerPoint export (new)

- `crates/slyde-pptx` writes a real OOXML deck by hand (no immature PPTX crate):
  content types, presentation/master/layout/theme, one full-bleed picture per
  slide, notes preserving the speaker view.
- Validated independently with `python-pptx` (`scripts/validate-pptx.py`):
  opens, correct slide count and EMU size, exactly one picture per slide, notes
  readable. The crate has 16 tests of its own, including a stack-based XML
  well-formedness checker.
- Wired into `POST /api/export?format=pptx`, `slyde export --format pptx`, the
  `export_presentation` MCP tool and the UI export menu.

### Rendering and export

- PNG export now runs **one** Chrome session for the whole deck over CDP
  (`Page.captureScreenshot`) instead of one process per slide, and renders at 2x
  (2560×1440 for 16:9). Every command and every slide has a hard timeout, and any
  failure falls back to the sequential CLI path so an unusual Chrome build cannot
  break export.
- PDF stays on `--print-to-pdf` (960×540 pt, exact 16:9).

### Backend (Rust workspace, 7 crates, 217 tests)

- `slyde-core` (54 tests): model, `PresentationState` with snapshot history and
  coalescing typing, validation, `.slyde` project format, element scoping,
  `insert_image` / `set_element_geometry`, `ImageRect`.
- `slyde-server` (126 tests): two listeners, all HTTP/WS endpoints, token +
  Origin guards, the deck library, the asset store, the copilot, exports.
- `slyde-mcp` (16 tests): JSON-RPC stdio with **concurrent dispatch** — each line
  becomes a task under a `Semaphore(8)`, one writer task serialises stdout, and a
  fast request overtakes a slow one.
- `slyde-pptx` (16 tests), `slyde-export` (4 tests incl. the CDP batch test),
  `slyde-app` (3 tests).
- Copilot: `POST /api/copilot/cancel` stops a turn server-side (the flag is
  checked per SSE chunk and per tool round, so the upstream response is dropped),
  and the round budget is 40 with the repeat guard intact.

### Frontend (`ui/`, vanilla ES modules, no bundler)

- Locale dictionaries live in `ui/static/locales/{en,es}.js` (263 keys each) and
  `i18n.js` is a 66-line loader with the same API. A parity test enforces equal
  key sets, no empty values, every `data-i18n*` attribute resolving, matching
  `{placeholder}` tokens, and that the dictionary did not creep back into
  `i18n.js`.
- `tests/ui/modules.test.mjs` imports every UI module in Node and fails on any
  `SyntaxError`: a duplicate `let` in a new module had produced a blank studio
  with no error in the product.
- Library sheet, image editing, PPTX menu item, all localized in EN and ES.
- Auth: JSON calls use `Authorization: Bearer`; only the WS handshake, downloads
  and `window.open` keep the query form.

### Persistence

- Autosave is debounced 300 ms and follows the **active** deck through a watch
  channel; a deck switch flushes the outgoing deck first.
- Ctrl+C flushes before shutdown. A server without a library still runs entirely
  in memory (tests rely on it).

### Verified end to end

- `cargo test --workspace`: 217 green.
- `node --test "tests/ui/*.test.mjs"`: 41 green, including the deck library flow,
  the image flow (insert → render in the sandbox → drag → geometry persisted →
  bundle download) and the 2x PNG export.
- `python scripts/validate-pptx.py target/tmp/pptx-check.pptx --expect-slides 3`:
  `OK … 3 slides, one full-bleed PNG each`.
- Live probe: `/s/{id}` 401 without a token, 200 with the sandbox token; the
  autosave writes into the active deck file (checked on disk).

## In Progress

Nothing partially implemented on the branch.

## Not Implemented Yet

- Copilot conversations are not persisted per deck; the thread lives in the page.
- OS keychain for provider keys (the file is owner-only on Unix; Windows has no
  DACL yet).
- `app.js` is still one large `boot()` closure, and the presenter logic is
  duplicated between `app.js` and `present.js`.
- Component-level DOM tests (the module smoke test is not a component runner).
- Deck thumbnails in the library (rows show title, slide count, age and theme).
- PPTX exports pictures, not editable text shapes.

## Known Issues / Bugs

- macOS and Linux remain unverified locally (the CI job runs the Rust suite on
  Linux, but that is not a manual pass).
- MSVC cannot link on this machine (Windows SDK missing); `x86_64-pc-windows-gnu`
  is the verified target and CI uses plain `stable` on Linux/Windows runners.
- Keyboard nudge inside the canvas needs the slide frame to own focus; clicking
  the picture takes focus, but a keyboard-first user has to click first.
- Image editing only exists in the studio stage, not in the presenter.
- The `.slyde` bundle duplicates assets that another deck may already have (no
  content addressing yet).
- Slide images must be raster; SVG and remote URLs are rejected by design.
- The inspector still rebuilds itself with `innerHTML`; the notes caret and
  scroll are restored explicitly, but focus can be lost while a remote change
  lands.
- Copilot: 40 rounds is still a budget, not a guarantee, and large decks may need
  a second "continue".
- PNG export at 2x triples the file size; there is no user-facing 1x/2x choice.
- Provider model lists include models that may not support tool calling; the UI
  shows metadata but does not pre-validate capability.
- Real provider calls are still unverified with live keys.

## Build & Platform Status

- Windows: **verified** with `cargo +stable-x86_64-pc-windows-gnu` (full suite,
  studio E2E, all exports). MSVC has no C++ build tools on this machine, so
  `scripts/cargo.ps1` now falls back to the GNU toolchain instead of failing.
- macOS / Linux: `slyde-core`, `slyde-pptx` and `slyde-export` are **cross-checked**
  for `x86_64-apple-darwin` and `x86_64-unknown-linux-gnu` and compile clean.
  `slyde-server` cannot be cross-built from Windows (`ring` needs a C toolchain
  for the target), so CI's ubuntu job is the real Linux verification. No manual
  pass on either OS yet.

## Backlog (Prioritized)

1. **Validate the copilot against live provider keys** (all eight).
2. **Split `ui/static/app.js`** into modules with a state object and per-region
   render, and share one presenter implementation with `present.js`.
3. **Persist copilot conversations** per deck and add token/cost estimates.
4. **OS keychain** for provider keys (Windows DACL first).
5. **Deck thumbnails** in the library sheet (one PNG per deck, cached).
6. **Editable PPTX** (text shapes) behind the current image-per-slide baseline.
7. Component-level DOM tests once a runner is chosen.
8. Content-addressed assets so two decks can share one image.

## Next Immediate Step

- Review the branch, then run the copilot with one real key and report any
  provider/model that fails.

## Completed

- [x] Deck library: files instead of a single autosave slot, endpoints, sheet,
      `⌘K` entries, migration of `current.slyde` — 2026-10-25
- [x] Images: asset store, paste/drop, move/resize, `.slyde` bundle v2 — 2026-10-25
- [x] PowerPoint export: `slyde-pptx` + API + CLI + MCP + UI, validated with
      python-pptx — 2026-10-25
- [x] PNG export in one Chrome session over CDP at 2x, with a CLI fallback and
      hard timeouts — 2026-10-25
- [x] MCP stdio loop dispatches concurrently under a semaphore with one writer —
      2026-10-25
- [x] Copilot: server-side cancel endpoint and a 40-round budget — 2026-10-25
- [x] i18n split per locale + parity test; module smoke test — 2026-10-25
- [x] Autosave path bug and stale-binary test bug fixed — 2026-10-25
- [x] Audit follow-up: cross-slide scoping, wire contract, sandbox token, undo
      coalescing, bearer auth, fmt/clippy/CI gate — 2026-10-25
- [x] In-app copilot, presenter, exports, `.slyde` persistence, MCP end-to-end,
      Geist + accessibility pass — 2026-10-24
