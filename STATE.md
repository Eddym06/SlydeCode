# STATE.md — Current State & Task Tracker

> Last updated: 2026-10-26 13:30

## Current Phase

Phase 3.3 — **product completion on `feat/product-completion`.** Rust 220 tests and
55 UI tests green (41 browser/CDP + 14 component); `cargo fmt --check` and
`cargo clippy -D warnings` clean.

This block closed the three product gaps the audit named — a real deck library,
real images, and PowerPoint export — plus the infrastructure that had been
missing: batch PNG rendering over CDP at 2x, a concurrent MCP stdio loop, a
server-side copilot cancel, per-locale dictionaries with parity tests, and a
module smoke test that would have caught the dead `app.js` import.

Recent rounds split the studio shell into modules: the presenter state machine
(the overlay and the standalone page had two copies that had drifted), the whole
inspector, the storyboard and the canvas (selection, HUD, inline editing and image
geometry). `app.js` went from ~2,600 to **~1,730 lines** and now holds only the
topbar, the stage frame, the agent-activity chip, the presenter overlay, the
palette actions and the boot glue.

Two data bugs were found and fixed in the previous round, both invisible to the
old suite:

- the autosave wrote into a placeholder `unset.slyde` instead of the active deck
  (`watch::Sender::send` silently drops the value when nobody is subscribed yet),
  so edits looked saved and were not;
- the UI suite was driving the previous build, because `findBinary()` preferred
  the gnu profile by list order instead of by recency.

## Implemented & Working

### Copilot efficiency and UI polish (2026-10-26)

- New copilot tools `build_deck` (create + every slide, notes and animations in
  one call) and `write_slides` (batch replace/append). A scripted-provider test
  drives the real loop: a six-slide deck takes **2 provider calls** (it used to
  take one per slide plus notes). `upsert_slide` clamps a guessed index instead
  of failing, and executed tool calls are stubbed in the resent history
  (`compact_history`), so later rounds no longer replay slide source.
- Bug: slide documents (opaque origin) could not load Geist from the sandbox port
  (CORS). `/fonts` and `/vendor` on the sandbox now send `Access-Control-Allow-Origin: *`.
- Bug: the shell grid had no column track, so at <1100px Export/Present were
  pushed off-screen and the copilot was hidden. Now the right panel is a
  slide-over drawer (topbar toggle, Esc/outside click closes) and the topbar
  collapses progressively down to 560px.
- Template icons are SVG (`template-icons.js`) instead of emoji; copilot
  suggestion chips follow the selection (block / slide / deck) and hide until a
  provider is connected.
- E2E notes: Chrome needs `--no-sandbox` as root (wrapper via `SLYDE_BROWSER`);
  the PDF export test fails on the untouched baseline in this container.


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
- `slyde-server` (128 tests): two listeners, all HTTP/WS endpoints, token +
  Origin guards, the deck library, the asset store, the copilot, exports.
- `slyde-mcp` (16 tests): JSON-RPC stdio with **concurrent dispatch** — each line
  becomes a task under a `Semaphore(8)`, one writer task serialises stdout, and a
  fast request overtakes a slow one.
- `slyde-pptx` (16 tests), `slyde-export` (4 tests incl. the CDP batch test),
  `slyde-app` (3 tests).
- Copilot: `POST /api/copilot/cancel` stops a turn server-side (the flag is
  checked per SSE chunk and per tool round, so the upstream response is dropped),
  and the round budget is 40 with the repeat guard intact.

### Shell modules

| Module | Responsibility |
|---|---|
| `presenter-core.js` | reveal state machine and key map, shared by the overlay and the presenter page |
| `inspector.js` | slide/deck tabs, animation timeline, notes, export buttons |
| `storyboard.js` | thumbnails, drag-and-drop, hover actions, roving tabindex |
| `canvas.js` | element selection, floating HUD, inline-edit messages, image paste/drop/geometry |
| `deck-library.js` | library sheet (list, search, rename, duplicate, delete) |
| `copilot.js`, `copilot-model-sheet.js` | provider sheet, thread, reasoning drawer |
| `agent-activity.js`, `teleprompter.js` | activity timing and speaker pacing (pure) |
| `api.js`, `i18n.js`, `locales/*` | HTTP/SSE client, translations |
| `app.js` | the shell: topbar, stage frame, chips, presenter overlay, boot |

### Inspector (own module)

- `ui/static/inspector.js` owns the slide tab, the deck tab and the animation
  timeline: it renders from the read model it is given and sends commands through
  injected callbacks, so it never touches the deck. `THEME_CHOICES`,
  `ANIMATION_KINDS` and the default easing live there (the palette imports the
  theme list from the same place).
- The DOM ids are unchanged (`#theme-select`, `#notes`, `#export-pdf`, …) because
  they are the contract with the export progress handler and the E2E suite; what
  changed is who owns the markup and the wiring.
- The notes caret/scroll restoration moved with it, so the fix cannot be lost in
  a future refactor of the shell.

### Presenter (shared)

- `ui/static/presenter-core.js` owns the reveal state (current slide, queued
  animation steps, frame readiness, intro-once) and the key map. The studio
  overlay (`app.js`) and the standalone page (`present.js`) both drive it, so
  "one step per advance", Home/End, `B`/`S`/`F`/Esc and the progress bar cannot
  disagree between surfaces. It is pure, and `tests/ui/components.test.mjs` covers
  it (intro once, wait-before-ready, slide advance, end, clamping, reload,
  malformed slides, every key).

### Copilot conversation

- One thread per deck, stored next to the keys (`copilot-threads.json`): bounded
  at 40 messages, 20k characters each and 100 decks. The panel restores it when
  it mounts and when the deck changes, "New chat" clears it, and every settled
  turn saves it. Tool rows and reasoning are not persisted: they were live UI, and
  replaying them next session would be a lie about what the model did.

### Frontend (`ui/`, vanilla ES modules, no bundler)

- Locale dictionaries live in `ui/static/locales/{en,es}.js` (263 keys each) and
  `i18n.js` is a 66-line loader with the same API. A parity test enforces equal
  key sets, no empty values, every `data-i18n*` attribute resolving, matching
  `{placeholder}` tokens, and that the dictionary did not creep back into
  `i18n.js`.
- `tests/ui/modules.test.mjs` imports every UI module in Node and fails on any
  `SyntaxError`: a duplicate `let` in a new module had produced a blank studio
  with no error in the product.
- `tests/ui/components.test.mjs` is the component layer the audit asked for: the
  DOM-free modules (reveal state, key map, progress, pacing, activity, image
  placement) plus the inspector's contract lists (12 kinds, 6 themes) are tested
  directly in Node.
  There is still no DOM component runner, which is the trade-off of the zero-npm
  decision.
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

- `app.js` is the shell in one closure (~1,730 lines): it is now glue (topbar,
  stage frame, chips, presenter overlay, boot), but it is still one closure with
  shared mutable state rather than a state object with per-region render.
- OS keychain for provider keys (the file is owner-only on Unix; Windows has no
  DACL yet).
- `app.js` is still one large `boot()` closure, and the presenter logic is
  duplicated between `app.js` and `present.js`.
- A DOM component runner (the pure modules are covered; the DOM wiring is only
  covered end-to-end).
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
- A restored copilot thread shows text only: tool rows and reasoning from the
  previous session are gone by design.
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
- `clippy -D warnings` is now clean on the Windows target **and** on the Linux
  cfg paths touched by the browser discovery work: the module is checked in
  isolation with `clippy-driver --target x86_64-unknown-linux-gnu` (see
  MEMORY.md for the recipe), because the full workspace cross-check stops at
  `ring`.

## Backlog (Prioritized)

1. **Validate the copilot against live provider keys** (all eight).
2. **Turn the shell into a state object**: the panels are modules, but the shell
   still shares ~30 mutable `let`s through closures instead of one read model
   with explicit subscriptions.
3. **OS keychain** for provider keys (Windows has a DACL now; a real keychain is
   next).
4. **Deck thumbnails** in the library sheet (one PNG per deck, cached).
5. **Editable PPTX** (text shapes) behind the current image-per-slide baseline.
6. A DOM component runner if the zero-npm rule is ever relaxed.
7. Content-addressed assets so two decks can share one image.
8. Token/cost estimates for copilot turns.

## Next Immediate Step

- Review the branch, then run the copilot with one real key and report any
  provider/model that fails.

## Completed

- [x] Fixed the first CI run: browser discovery now builds its candidate list
      per OS, so no platform leaves a dead `mut` (`unused_mut` under
      `-D warnings` on Linux); Linux also gained absolute snap/flatpak paths, and
      a test guards against an empty candidate list — 2026-10-25
- [x] CI is green on the pull request (Linux fmt/clippy/tests and the Windows
      studio suite, 55/55). The canvas E2E tests that failed three runs in a row
      were racing the stage document: they now wait for `data-slyde-ready`
      through the iframe's CDP target, tolerate the lazily created HUD node,
      poll for the outgoing command and fall back to clicking the block through
      the stage session when parent-page input misses the frame — 2026-10-25
- [x] Canvas extracted (selection, HUD, inline editing, image placement) and the
      storyboard extracted (thumbnails, drag-and-drop, hover actions) — 2026-10-25
- [x] Inspector extracted to its own module (markup, animation timeline, notes
      caret, export wiring) and the contract lists tested — 2026-10-25
- [x] Shared presenter state machine + component tests — 2026-10-25
- [x] Copilot conversations persisted per deck (bounded), restored on mount and
      deck switch, cleared by New chat — 2026-10-25
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
