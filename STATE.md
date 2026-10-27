# STATE.md — Current State & Task Tracker

> Last updated: 2026-10-27 13:10

## Current Phase

Phase 3.8 — **`feat/product-completion` integrated into the post-PR #3 `main`** (branch
`integrate/product-completion`, not pushed). 160 server tests plus the rest of the Rust workspace,
90 UI tests (browser/CDP + component + module) green; `cargo fmt --check` and
`clippy -D warnings` clean on `x86_64-pc-windows-gnu`.

### What the integration kept, and what it dropped

Both branches had built some of the same things. Where they overlapped, `main` won (it is the
reviewed, merged line) and only the non-overlapping parts of `feat` were re-applied.

Kept from `feat`:

- **Provider keys in the OS credential store** (`keyring` 3.x; owner-only file as fallback).
- **`set_title` copilot tool**, agent title validation, EN/ES labels.
- **PNG quality** (`scale=1..3` on `/api/export`, inspector select persisted in `localStorage`).
- **Standalone `.html` export** (`render_standalone_presentation`, inspector tile, top-bar item).
- **Presenter laser and pen** (`L` / `P` / `C`) in the studio overlay and `/present`, via
  `presenter-draw.js`. Browser shortcuts (Ctrl/Cmd/Alt + letter) are no longer read as presenter
  commands.
- **Sandbox hardening** (`event.source === window.parent`, `findById` instead of splicing ids).
- **Geometry edits lift `max-width`/`max-height`** so a deliberate resize is not clamped.
- **Magic redesign** button in the inspector (with an ES/EN prompt; `feat` had no ES copy).
- **Test harness pins Chrome to `--lang=en-US`**: three tests used to fail on a Spanish machine.

Dropped from `feat` (superseded by an equivalent on `main`, or incompatible with it):

- Editable PowerPoint text boxes: `main`'s `PptxText` runs implementation stays. `slyde-pptx` and
  `slyde-export` are byte-identical to `main`.
- PNG library covers (`/api/decks/thumbnails`): `main` renders covers as live iframes.
- Universal canvas gizmo: re-done on top of `main`'s canvas (see "Canvas handles" below)
  instead of taking `feat`'s version, so multi-selection and align/distribute are untouched.
- `app.css` split into 8 sheets and the `app.js` shell split (`presenter-overlay.js`,
  `agent-activity-view.js`): both conflict with the layouts, wizard, versions and transitions that
  `main` added to the same files.

`feat/product-completion` itself is untouched on origin.

### Canvas handles (2026-10-27)

Eight resize handles around a single selected block, plus magnetic guides while moving or
resizing. Lives in the slide runtime (`sandbox.rs`) and reuses `main`'s drag (4 px threshold,
freeze to absolute, `image-geometry` to the studio), so no new message type and no change to
`canvas.js`. Snaps within 6 stage px to the stage edges and centre, a 64 px safe margin and the
edges and centres of the other blocks; holding Alt turns snapping off. A block cannot shrink below
16 px. A multi-selection has no handles and still moves as a group. `tests/ui/canvas-handles.test.mjs`
drags a handle with real mouse events and checks that the edge lands exactly on the stage centre
and that the minimum size holds. Not done: aspect-ratio lock (Shift) and rotation.

### Windowless launcher (2026-10-27)

Double-clicking used to open a terminal that also *was* the server. Now two executables share the
`slyde-app` crate (lib + two bins):

- `slyde.exe` (console): CLI and MCP, unchanged; the server it starts runs until Ctrl+C.
- `slydecode.exe` (GUI subsystem, no console): what a user opens. It passes `--exit-when-closed`.
  The server counts connected windows through the WebSocket (`slyde_server::presence`) and stops
  8 s after the last one disconnects, or 45 s after start if no window ever connected. A reload or a
  switch between editor and presenter reconnects inside the 8 s. It logs to
  `<app-data>\slydecode.log` (trimmed past 512 KiB) and shows a message box when it cannot start
  (no browser, panic).

Verified on the real binaries from the unzipped package: no `conhost` child, the app window opens,
a second launch reuses the running server and exits, closing the window stops the server in 8 s and
clears `runtime.json`, and `slyde.exe mcp` still answers `initialize` and `tools/list` (11 tools).
`scripts/package-windows.ps1` builds both and writes `dist\SlydeCode\` plus a zip.

Still open: an icon for `slydecode.exe` (needs a resource compiler at build time and a larger icon
than the 32 px `favicon.ico`), code signing, and an installer. Without a signature Windows
SmartScreen warns on first run.

### Release binary check (2026-10-27)

`cargo build --release` (GNU target) gives a 21 MB `slyde.exe`. It does **not** embed the UI: it
reads `ui/` and `assets/` from beside the executable (`resolve_ui_dir`, `resolve_assets_dir`), so a
distributable is the folder `slyde.exe` + `ui/` + `assets/` (built under `dist/`, git-ignored).
Started from an unrelated working directory with an isolated `SLYDE_DATA_DIR`, it served 50 files
(`/`, `/present`, 41 under `/static`, the fonts) byte-identical to the repo, and the full UI suite
(90 tests) passed with `SLYDE_BIN` pointing at it. Embedding the UI in the binary is still open.

**Double-click bug (found 2026-10-27):** `slyde.exe` with no arguments panicked at `&args[1..]`
on an empty list, so the release binary never opened when launched like a user would. Every test
passed `app` explicitly, which is why nothing caught it. `split_command` now treats a bare launch
and a leading option (`slyde --no-open`) as `app`, and has unit tests. Re-checked by launching
the release binary with no arguments: the server stays up, answers 200, and Chrome opens the app.

Phase 3.5 — **Real Interaction & Stability Test Suite (100% Green).**
All 225 Rust tests (7 crates), 32 component/module unit tests, 24 Studio E2E tests, and 17 Real Interaction & Stability E2E tests (`tests/ui/interaction.test.mjs`) passing with zero failures and zero warnings across the entire workspace.

### Implemented & Verified in this round:
- **Universal Canvas Transform Gizmo & Snapping**: Any DOM block (text, cards, images, diagrams) can be freely moved and resized with 8-point handles, magnetic guide snapping, and arrow-key nudge (1px / 10px with Shift).
- **Canvas Pan & Smooth Zoom**: Space + drag for smooth stage panning and Ctrl + mouse wheel continuous zoom.
- **Full Keyboard Editing Suite**: `Delete`/`Backspace` slide deletion, `Cmd+D`/`Ctrl+D` slide duplication, `Cmd+C`/`Cmd+V` slide copy/paste (`UiCommand::PasteSlide`), and `F5` / `Cmd+Enter` / `Cmd+Shift+Enter` instant presenter launch.
- **Presenter Live Laser & Pen Tools**: Puntero láser virtual (`L`) con halo de degradado de marca y lápiz de anotación en directo (`P`) con borrado instantáneo (`C`) y sincronización automática de resolución HiDPI.
- **Standalone Web Deck Export (`.html`)**: `slyde export --format html` y botón en UI para exportar un único archivo HTML offline 100% interactivo y autocontenido sin servidor ni conexión.
- **Copilot `set_theme` & Magic Redesign**: Nueva herramienta para cambiar paletas en caliente y botón en el inspector para rediseñar diapositivas con IA.
- **End-to-End Real Interaction & Stability Suite (`tests/ui/interaction.test.mjs`)**: 17 tests reales con Chrome CDP cubriendo manipulación con ratón/teclado, concurrencia, prevención de fugas DOM y estabilidad del backend.

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
- Copilot turns report `usage` (provider calls + ~input tokens, estimated at
  4 chars/token) and the thread shows it under the finished message.
- SVG brand mark (`logo-icon.svg`, also the favicon) replaces the PNG in the
  topbar; a keyboard shortcut sheet (`?`, `shortcuts.js`) lists studio, canvas
  and presenter keys in EN/ES.
- **Right column redesign.** "Inspector" is now **Design** (ES "Diseño"). A
  sliding spring-physics switch (Design | Copilot) replaces the tab strip, the
  Slide/Deck sub-switch shares the same mechanism, and panels cross-slide in the
  direction of the thumb. Design is a stack of cards (layout, notes, a motion
  timeline with numbered steps, an elements tree, source), the Deck tab has
  mini-slide theme previews, aspect-ratio shapes and export tiles. Cards animate
  in only when the slide or tab changes, never on a remote re-render. The
  copilot got gradient user bubbles, a live avatar, an animated work log (spinner
  to drawn check), a floating empty state, horizontally scrolling prompt chips
  and an elevated composer with a morphing send/stop button. Styles live in
  `ui/static/panels.css` (loaded after `app.css`); all ids/classes the scripts
  and E2E rely on are unchanged. Reduced-motion is honored.
- **Backlog items 2-12 delivered.**
  - Copilot: real token usage from the providers (OpenAI-compatible
    `stream_options.include_usage`, Anthropic `message_start`/`message_delta`),
    with a size estimate when a provider reports none; live write progress
    (`progress` events with the slide count); one-step **Undo AI changes**
    (`POST /api/copilot/revert`, valid only while the deck is exactly as the
    turn left it); ArrowUp recall of earlier prompts.
  - Shell: File and View menus (keyboard navigable, `menus.js`), zoom shortcuts
    (Ctrl/Cmd +/-/0), palette entries for the new actions.
  - Storyboard: Ctrl/Shift multi-select with a bulk bar (one undoable
    `remove_slides`), Delete/Ctrl+D/Alt+Arrows, grid view, entrance animation.
  - Layout picker previews wear the active theme (`layout-previews.js`); theme
    swatches use the real theme colors.
  - Source editor (`code-editor.js`): line numbers, HTML/CSS highlighting,
    debounced `set_slide_code`, survives re-renders and keeps the caret.
  - Accessibility: `focus-trap.js` on the library, palette, model sheet and
    shortcut sheet; drawer focus handling; menu keyboard pattern.
  - Library covers: `GET /d/{deck}` on the sandbox renders the first slide of any
    deck (no browser launched); the sheet keeps one iframe per deck revision.
  - **PowerPoint with editable text**: Chrome reads every text block (box, runs,
    color, size, alignment, line spacing) over CDP, hides it and screenshots the
    rest; `slyde-pptx` writes real text boxes over that picture. Code, math and
    diagrams stay in the picture. Verified with python-pptx and LibreOffice.
  - Tests: Rust 56 core / 138 server / 16 pptx / export extraction test against
    real Chrome; UI `panels.test.mjs` (8 flows) plus component tests for the
    highlighter, focus trap and layout previews.
- **UI review round (2026-10-27).**
  - Slides written without a template are labeled from their markup
    (`TemplateType::infer`, new `Custom` variant) instead of "Code"; the template
    is optional in the copilot and MCP tools.
  - The element list says "Title", "Paragraph", "List · 3 points"; tags and
    handles only exist in Developer mode. The canvas toolbar names the block the
    same way. The deck title keeps its room in the top bar.
  - **Copilot waiting UX:** a work card with the real phase (waiting, thinking,
    "writing slide N", applying), an elapsed timer, the model's latest line of
    thought, a segmented track when the user named a slide count, and shimmering
    storyboard placeholders. Reasoning is coalesced server-side (120 ms), the
    drawer starts collapsed, and **slides are applied while the model is still
    streaming** (`copilot/partial.rs`; `build_deck` must list title/theme/ratio
    before slides, which the prompt asks for).
  - **Canvas:** a formatting toolbar (bold, italic, size, color, align, bullets),
    multi-select (Shift/Ctrl-click) with align/distribute (one undo step), any
    block can be dragged once selected, arrow-key nudge for every block, and a
    keyboard model for the canvas region (Enter, Tab, arrows, Escape) with
    screen-reader announcements.
  - **Content tools:** elements library (shapes, icons, charts, tables), slide
    layouts that keep the slide's words (and remember what a layout hid) with
    "use on every slide", slide transitions (none, fade, slide, push, zoom, flip)
    played by both presenters, and named versions (automatic checkpoint before
    every copilot turn that changes the deck; restore is undoable).
  - Not done: grouping blocks (needs a DOM-level model of the slide code) and a
    live render of the half-written slide. DeepSeek could not be reached from this
    environment (network policy), so the copilot was verified against a scripted
    provider only.
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

### First-run experience and live draft (2026-10-27)

- **Welcome wizard** (`ui/static/welcome.js`): opens once for an untouched deck
  (`localStorage` flag `slyde.welcomed`), reopenable from File menu and the
  command palette. Three paths: describe an idea, template gallery, example.
  The AI path connects a provider in the same step (`copilot.connect()` saves the
  key, `onboarding-models.js` picks a writing model, the topic goes to the
  copilot).
- **Deck templates** (`deck-templates.js`): five bilingual, complete stories,
  rendered through the Layout card's `renderLayout` and sent as one
  `apply_template` command (core `apply_template`, one undoable step).
  Figures are placeholders (`00%`), never invented data.
- **Live draft** (`partial::draft_html` -> `ChatEvent::Progress.draft` ->
  `live-draft.js`): the markup of the slide being written is painted in a
  scripts-off, network-off (`sandbox=""` + CSP) frame over the canvas and fades
  when the turn ends or goes quiet for 4.5 s.
- E2E helper: `newPage(url, { welcomed })` marks the wizard as seen by default so
  unrelated suites are not covered by it; `tests/ui/welcome.test.mjs` opts out.

## In Progress

Nothing partially implemented on the branch.

## Not Implemented Yet

- `app.js` (~1,440 lines) is still one `boot()` closure for what the shell itself owns
  (`view`, selection, stage key, zoom, panels). A store is only worth it if that grows.
- The overlay presenter in the studio and the standalone `/present` page share the reveal
  state machine, but each still has its own DOM wiring.
- A DOM component runner (the pure modules are covered; the DOM wiring is only
  covered end-to-end).
- Deck thumbnails in the library (rows show title, slide count, age and theme).
- PPTX text uses the deck font name; PowerPoint substitutes it when it is not installed. Gradient text becomes the accent color.
- Cost estimates (price per token) are not shown: prices change per model.

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
- Provider model lists include models that may not support tool calling; the UI
  shows metadata but does not pre-validate capability.
- Only DeepSeek has been exercised with a live key. OpenAI, Anthropic, Gemini, xAI, Groq,
  OpenRouter and NVIDIA are still unverified.
- A provider connection reset mid-stream (seen once with DeepSeek, `os error 10054`) ends the
  turn with a localized network error; there is no automatic retry.
- A first copilot turn takes 70–140 s on DeepSeek and streams ~8–10k `thinking` SSE frames.
  Coalescing thinking deltas would cut traffic without changing the UI.
- `studio.test.mjs` is ~2,200 lines and `interaction.test.mjs` still uses many fixed
  `sleep()` waits; one of them already flaked. Convert them to readiness waits as they bite.
- PowerPoint output is validated with `python-pptx` only; a missing Geist typeface makes
  PowerPoint substitute another one (glyph widths change, line breaks do not).
- The tests leave hundreds of `slyde-*` folders in `%TEMP%`.

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

1. **Validate the remaining copilot providers** (seven left) and add one retry for a
   connection reset that happens before the first delta.
2. Open the exported `.pptx` in real PowerPoint/Keynote/Google Slides and compare it
   with the presenter; tune the line spacing if the baseline sits off.
3. Linux credential store, if a `libdbus` dependency ever becomes acceptable.
4. Clean the test temp folders (each suite should remove its own).
5. Editable PowerPoint shapes for diagrams and code blocks.
6. A DOM component runner if the zero-npm rule is ever relaxed.
7. Content-addressed assets so two decks can share one image.
8. Token/cost estimates for copilot turns.

## Next Immediate Step

- Review the branch, open an exported deck in real PowerPoint, and run the same live
  probe against the other seven providers.

## Completed

- [x] New brand mark shared by the app icon, favicon, copilot glyph and README
      (light/dark banner) — 2026-10-27

- [x] First-run wizard, five full deck templates, one-step AI connect and the
      live slide draft while the copilot writes — 2026-10-27

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
