# MEMORY.md — SlydeCode Knowledge Base

> Durable knowledge only. Progress, task breakdowns and status belong in `STATE.md`.
> Threshold for writing here: *will this still matter in a month, and would rediscovering it waste real time?*

## Architectural Decisions

| Date | Decision | Alternatives Considered | Rationale |
|------|----------|--------------------------|-----------|
| 2026-10-24 | The engineering charter (`AGENTS.md`, mirrored as `CLAUDE.md`) is the binding process for all agent work | Ad-hoc per-session instructions | Continuity and a stable quality bar across sessions and agents |
| 2026-10-24 | Split project memory into `MEMORY.md` (durable) and `STATE.md` (live progress + task tracker) | Single memory file; three files (`MEMORY.md` / `ESTADO.md` / `TASK.md`) | `MEMORY.md` must stay short and worth rereading, so it holds only essential decisions and gotchas. `STATE.md` absorbs all progress and the task tracker, which keeps the state accurate without polluting the memory |
| 2026-10-24 | **Chrome is the only rendering surface.** The Rust core is a local HTTP/WS server plus an MCP stdio server; the UI is served over `127.0.0.1` and displayed in the user's Chrome. No embedded web engine in v1 | Wry/native WebView; CEF; Ultralight 2.0; Servo embedding | The product requires that MCP-driven sessions open Chrome and show the deck there. Once Chrome is the audience surface, only Chrome guarantees that what the user approves in the editor is what the audience sees. It also removes all C++/WebView toolchains, keeps our binary small and our processes under the RAM budget, and lets the export pipeline reuse the same engine |
| 2026-10-24 | Design system: warm minimalism ("Studio"), violet→coral signature gradient, Geist Sans + Geist Mono, one signature moment (the generation sweep). Full spec in `docs/design-system.md` | Monochrome Swiss; dark-first neon; playful/illustrated | The brief fixes the palette and rejects terminal aesthetics. Warm minimalism lets the product feel premium without competing with the user's own slides, and keeps the interface quiet so the generated content is the hero |
| 2026-10-24 | **MCP server is a thin HTTP proxy** into the running studio (`/api/agent`), auto-spawning `slyde app` when no healthy runtime exists | MCP server owning its own `PresentationState`; MCP writing files the app watches | One state, one code path. The agent and the human see the same deck, undo/redo works across both, and there is no sync protocol to get wrong. Auto-spawn keeps `slyde mcp` usable from a bare MCP client config |
| 2026-10-24 | **Hand-rolled JSON-RPC 2.0 stdio loop and hand-written JSON Schema** for the 7 tools, instead of the `rmcp` SDK | `rmcp` (official Rust SDK); `mcp-core`; writing a generic schema generator | The tool surface is a public, versioned contract (§12) and is small and stable. Hand-written schemas are readable, reviewable and independent of an SDK whose API was not verified for the exact protocol revision. Migration to `rmcp` stays open behind the same dispatch function |
| 2026-10-24 | **Export drives the Chrome CLI** (`--print-to-pdf`, `--screenshot`) rather than CDP | CDP `Page.printToPDF` / `Page.captureScreenshot`; a PDF crate | No CDP client, no WebSocket dependency, no protocol version drift: one process spawn per export. `@page { size: 1280px 720px; margin: 0 }` makes the page size exact (verified: 960×540 pt). CDP remains the fallback if per-page control is ever needed |
| 2026-10-24 | **`.slyde` is plain, versioned JSON** (`slydecore`, version 1) holding the whole presentation | ZIP container with assets; MessagePack; SQLite | A deck is text (HTML/CSS per slide), so JSON keeps it diffable, inspectable and repairable without a tool. A ZIP only pays off once slides can embed image assets, which the sandbox CSP currently forbids anyway (`img-src data: blob:`) |
| 2026-10-24 | **Persistence = autosave to app-data + explicit download/upload**, not a native save dialog | Native file dialog via a desktop shell; only autosave; File System Access API | The studio is a web page inside Chrome, so the browser-native primitives are a download link (`GET /api/deck`) and a file input (`POST /api/deck/open`). They work on every platform and need no shell. Autosave is always on, so the common case (close and reopen) never asks the user to save |
| 2026-10-24 | **Typefaces are self-hosted.** Variable Geist Sans/Mono woff2 files live in `assets/fonts/` and are served at `/fonts` on the app origin **and** the sandbox origin | Google Fonts CDN; system font stack only; loading the npm package at runtime | Offline determinism and brand fidelity: no third-party request from a `127.0.0.1` app, no FOUT caused by a CDN, and slide exports embed the same face the editor shows. Serving from the sandbox origin lets every slide iframe share one cached copy instead of inlining ~70 KB per document |

## Library Research Log

### Rendering surface — DECIDED 2026-10-24 (Chrome only, no embedded engine)

Constraint that reframed the problem: when SlydeCode is driven from MCP, it must open Google Chrome and load the interface there to show the presentation. WYSIWYG between editor and audience then becomes a hard requirement, not a nice-to-have.

Candidates evaluated:

| Option | Rendering parity with Chrome | Footprint | License / cost | Maturity | Verdict |
|---|---|---|---|---|---|
| Chrome / Chromium as the surface (`--app` / `--kiosk`, state over HTTP+WS) | Exact — it *is* Chrome | 0 MB added to our binary; our Rust process stays small | Free | Maximum (it is the reference engine) | **Chosen** |
| Wry 0.57.x (WebView2 / WKWebView / WebKitGTK) | Diverges on macOS and Linux; matches on Windows only | Small binary, uses OS webview | Apache-2.0 / MIT | High, actively maintained (Tauri) | Kept as a *future shell* option, never as the only renderer |
| CEF (Chromium Embedded Framework) | Exact (same engine) | ~100–200 MB binary, C++ toolchain, long builds | BSD-style | High but heavy | Rejected: contradicts "lightweight", slow iteration |
| Ultralight 2.0 | Different engine: WebKit fork, single-process | 9–17 MB RAM, small binary | Free (<$200K turnover **and** <$200K funding), Indie $395/title (<$1M), Pro $4,500/title (<$10M), Enterprise custom. WebCore portion LGPL; full source only Enterprise. Rust bindings are community-maintained | Niche (games/embedded) | Rejected: the deck would look different in the editor than in the Chrome window the audience sees, which is the exact failure the product cannot afford |
| Servo embedding (`servo` crate 0.7.0, Oct 2026) | No | — | MPL-2.0 | Not consumable as a webview library yet; the Servo Book still points to `tauri-runtime-verso` / `servo-gtk` for embedding | Rejected for now; revisit in a year |

Why Chrome wins, concretely:

1. **One engine, one truth.** Editor preview, presenter and PDF export all render the same slide HTML with the same engine, so there is no "looked fine in the app, broke on the projector" class of bug.
2. **The requirement already forces it.** Supporting two engines would mean a permanent test matrix and visual QA pass for every slide template.
3. **Small and fast.** No Chromium or WebKit ships in our binary; startup is a process spawn, not an engine init. Our own footprint is a Rust server, not a browser.
4. **Exports come cheap.** The same CDP connection that drives the presentation produces the PDF.

#### Implementation notes that matter (verified against Chrome/Chromium docs and the DevTools protocol)

- **Headless is unified since Chrome 112.** `--headless` is the new headless; the old implementation only survives as the separate `chrome-headless-shell` binary (Chrome 132+). Use `--headless` for export, never `--headless=old`.
- **`Page.printToPDF`** (CDP): `paperWidth`/`paperHeight` are in **inches**, margins default to ~1 cm, `printBackground` defaults to false, and `preferCSSPageSize: true` is what makes `@page { size: 1280px 720px; margin: 0 }` authoritative. Without it, Chrome scales content to Letter/A4 and adds margins.
- **`Page.captureScreenshot`** supports `png`/`jpeg`/`webp`, `clip`, `captureBeyondViewport` and `optimizeForSpeed`. This is the fallback for PNG-per-slide export and for thumbnails in the storyboard.
- **`--app=<url>`** forces application mode (no tabs, no omnibox) and, per Chromium's switch docs, may force installation as a PWA. Verify on the target Chrome versions; fallback is `--new-window` plus a JS fullscreen request, and `--kiosk` for presenting.
- **`--start-fullscreen`** behaves like F11; `--kiosk` is the real presentation mode.
- **Single instance:** launching with the same `--user-data-dir` while Chrome is already running attaches to that instance. Deliberate choice still pending: reuse the user's default profile (familiar, instant) vs. a dedicated profile under app-data (isolated, no PWA pollution). Default to the user profile, expose a flag.
- **Browser discovery order:** Chrome → Chromium → Edge → Brave (all Chromium-based and support the same flags). If none exists, fail with an actionable message; do not silently degrade.

#### Risks / things to revisit

- Chrome is an external dependency: it can update under us. Pin the launch to a detected binary and record its `--version` in logs; features must degrade gracefully.
- Rendering is only identical if slides avoid Chrome-version-specific CSS. Treat current stable Chrome as the target baseline and state it.
- If native app identity (own icon, dock behaviour, no browser process in the taskbar) becomes a product blocker, add a Wry shell **behind the existing HTTP boundary** — core, tools and UI stay untouched.
- The 100MB RAM budget applies to SlydeCode's own processes; Chrome's memory is the user's browser and is explicitly out of scope (documented in the charter, §9).

### Typography — DECIDED 2026-10-24 (bundled variable Geist)

Candidates evaluated:

| Option | Offline / privacy | Determinism in export | Footprint | License | Verdict |
|---|---|---|---|---|---|
| System font stack only | Yes | No — each OS renders differently | 0 | n/a | Rejected: breaks brand and WYSIWYG |
| Google Fonts CDN `<link>` | No, requires network | No — first paint may use fallback | 0 locally | OFL | Rejected: a localhost studio should not call a third party, and offline demos must work |
| **`geist` npm 1.8.2, self-hosted woff2** | Yes | Yes | ~293 KB for four variable faces | SIL OFL 1.1 | **Chosen** |
| Inter variable from `rsms/inter` | Yes | Yes | Similar | SIL OFL 1.1 | Fallback only; loses the current brand face |

Files kept: `Geist-Variable.woff2`, `Geist-Italic-Variable.woff2`, `GeistMono-Variable.woff2`, `GeistMono-Italic-Variable.woff2`, plus `LICENSE-Geist.txt`. The package was extracted from the npm tarball (not a random mirror), so the license and the binaries come from the same release.

Risks / things to revisit: the repo grows by ~293 KB; the sandbox CSP must keep `font-src 'self' data:` and the sandbox router must mount `/fonts` or slides silently fall back; if Geist ever becomes unavailable, the documented fallback stack (Inter, system-ui) keeps the product usable.

### PPTX export — OPEN, not yet decided

- Candidates: `ppt-rs` 0.2.37 (Oct 2026, ~100k downloads, active) — most promising; `pptx` 0.1.0 (Feb 2026, single release, low adoption); `office_oxide` (multi-format, commercial-ish positioning).
- Not validated yet: none of these guarantees fidelity for our generated layouts, and PowerPoint/Keynote are the judge, not the crate's test suite.
- Plan: ship PDF first (`Page.printToPDF`), then add PPTX behind an `Exporter` trait and validate real files in PowerPoint, Keynote and Google Slides before promising it.

## Known Gotchas & Constraints

- `CLAUDE.md` is a generated mirror of `AGENTS.md`. Edit `AGENTS.md` first, then regenerate the mirror; never edit both by hand.
- `CLAUDE.md` ends with the line "No leas el AGENTS.md": Claude sessions must read only `CLAUDE.md`, so the two copies never diverge in-context. Non-Claude agents read `AGENTS.md`.
- Renaming `AGENTS.md` breaks that instruction — update `CLAUDE.md` in the same change.
- The local server is reachable by anything on the machine, and by any web page via `localhost`. Bind to `127.0.0.1`, use an ephemeral port, require a per-session token in both the URL and the WS handshake, and validate the `Origin` header. No `Access-Control-Allow-Origin: *`.
- A second MCP client must not spawn a second app. Use a rendezvous record (lock file in the OS app-data dir with port + token) so later instances reuse the running server instead of opening another Chrome window.
- The application must never expose code, JSON-RPC logs or technical jargon to non-technical users outside an explicit Developer Mode.
- **Slides are AI-generated code and must be treated as untrusted.** Render each slide in a cross-origin sandboxed iframe (`sandbox="allow-scripts"`, served from a different origin/port than the app shell). Never combine `allow-scripts` with `allow-same-origin`: that grants the slide the app's origin, its session token and its API. Slide CSS must never leak into app chrome (and vice versa).
- The master prompt (§1) still asks for Ultralight or Wry as the rendering engine. That conflicts with the approved Chrome-only decision; the Chrome decision wins until the user explicitly reverses it. Do not "fix" the architecture back to an embedded engine on the strength of that line.
- **Slide documents must scale themselves.** `window.innerWidth/innerHeight` inside a freshly created iframe can still be `0×0` when the load event fires; scaling to `0` makes the slide invisible until the next resize. Guard against zero and re-run `fit()` from a `ResizeObserver` on `documentElement`.
- Do **not** center the 1280×720 stage with CSS grid inside a small iframe: the oversized grid item is centered against a 1280-wide container, which pushes the content outside the viewport. Keep the stage `position: absolute; top: 0; left: 0` and translate it by `(viewport - scaled size) / 2` in JS.
- In presenter mode animated targets start at `opacity: 0` and only appear when a step fires. Any screenshot or export path must use `?static=1` (or trigger the steps) or it will capture an empty slide.
- Chrome's `--print-to-pdf` / `--screenshot` resolve relative output paths unpredictably and can fail with "path not found" even when the directory exists. Always pass an absolute path (`std::path::absolute`).
- Chrome exits with code `0` even when it failed to write the output file. Verify the artifact exists instead of trusting the exit status.
- On this machine the MSVC target cannot link (Windows SDK missing, `LNK1181: kernel32.lib`). Build with `cargo +stable-x86_64-pc-windows-gnu`. Revisit once the SDK is installed, since GNU is not the shipping target.
- `cargo build` fails with "Acceso denegado (os error 5)" while `slyde.exe` is running: the debug binary is locked. Stop the server before rebuilding.
- PowerShell `Set-Content -Encoding utf8` writes a BOM and re-encodes piped native output as UTF-16. Use `[IO.File]::WriteAllText` with `UTF8Encoding($false)` and `cmd /c "... > file"` when testing the MCP stdio stream.
- **Never autosave over a deck file that failed to open.** If the saved file exists but does not validate, disable autosave for that session and report it; otherwise the first edit silently destroys the user's recoverable work.
- Deck saves are atomic (write `*.slyde.tmp`, then rename). `std::fs::rename` replaces an existing file on Windows as well as Unix, so no delete-before-rename dance is needed.
- `Content-Disposition` header values must be ASCII: use the RFC 5987 `filename*=UTF-8''<percent-encoded>` form for real deck names and keep a plain ASCII `filename` fallback.
- `axum`'s default body limit is 2 MB; the `.slyde` upload route sets its own `DefaultBodyLimit` because a legitimate deck can exceed that.
- **Fonts need both origins.** A slide document is served from the sandbox listener, so `/fonts` is mounted on the app router *and* the sandbox router, and the sandbox CSP must include `font-src 'self' data:`. If either is missing, the deck still renders but silently falls back to system fonts — no error, just a brand regression.
- **`outline: none` in any `:focus` rule beats the global `:focus-visible` ring** because of specificity. Text fields and selects keep their border change *and* the ring; never re-add `outline: none` without an equally visible replacement.
- `document.fonts.check()` can report `false` before `await document.fonts.ready`; the UI E2E test waits for readiness before asserting the face loaded. `fonts.check()` only covers fonts the document actually requested, so check a family the page uses.
- The UI E2E suite is dependency-free: Node's built-in test runner plus a tiny CDP client (`scripts/lib/cdp.mjs`). Run it with `node --test "tests/ui/*.test.mjs"`; a bare directory argument is treated as a module path by Node 24. It spawns the built binary with `SLYDE_DATA_DIR` pointing at a temp dir, so it never touches the user's real deck or Chrome profile.
- Tab order in a browser continues from the last focused element, not from the top of the document. Tests that assert focus must focus a known control first; `document.body.focus()` after blurring is not a reliable reset.

## Design System Reference

- Canonical spec: `docs/design-system.md`. Visual reference: `docs/reference/slydecode-concept.html` (mockup only, not the product UI).
- Core tokens: `--bg`/`--surface`/`--ink` neutrals per mode, `--brand` `#7C3AED` (dark: `#9B6BFF`), `--accent` `#FF6B4A` (dark: `#FF8265`).
- Signature gradient: `linear-gradient(135deg in oklch, #7C3AED, #FF6B4A)`. Decorative only: logo, AI chip, sweep, rings, display type ≥24px. Never behind small text (`#FF6B4A` on white is ~2.8:1).
- Primary buttons use `--gradient-action` (`#6D28D9 → #7C3AED`, white text ≥5.7:1); coral appears only in hover glow.
- Type: Geist Sans (bundled variable woff2; fallback Inter), mono only in Developer Mode. Base 15px, `display` 56, sentence case, no all-caps labels.
- Accessibility baseline: `--ink-subtle` was darkened to `#6F6B73` (light) / `#908D95` (dark) to clear AA on both panel surfaces; the E2E suite measures six text/token pairs per mode and fails under 4.5:1.
- Motion: 120/180/260/420/700ms with `--ease-out: cubic-bezier(.16,1,.3,1)`; transform/opacity only; exits at 0.7×; reduced-motion degrades to opacity.
- The single signature moment: generation sweep + one anchored micro-label + 40ms staggered reveal. No spinner for generation, ever.

## Superseded Decisions

- Three-file memory system (`MEMORY.md` + `ESTADO.md` + `TASK.md`), superseded on 2026-10-24: three files meant three reads per task, duplicated update work and allowed contradictions between state and tracker. Replaced by `MEMORY.md` + `STATE.md`.
- PDF export through the CDP `Page.printToPDF` call, superseded on 2026-10-24 by the Chrome CLI (`--print-to-pdf`): the CLI needs no WebSocket client or protocol version, and `@page size` already yields the exact page size. CDP stays as the fallback if per-page control is needed.
- "Lightweight native rendering engine" as an embedded component (Wry/Ultralight/Servo), superseded on 2026-10-24 by Chrome as the only rendering surface; the shell is now a swappable launcher.