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
| 2026-10-24 | **Exports run in-process from the studio**: `POST /api/export` snapshots the deck and drives headless Chrome through `slyde-export`. PDF returns bytes; PNG returns a store-only ZIP with one image per slide. Progress is pushed over the existing WS event stream | Shelling out to `slyde export`; browser-side ZIP (`CompressionStream`); N separate PNG downloads | One pipeline shared by UI, CLI and MCP; no self-spawn or path guessing; the browser gets exactly one download per format, named after the deck. Snapshotting the deck before rendering means an export never holds the state lock while Chrome runs |
| 2026-10-24 | **AI activity is inferred from `/api/agent`**, broadcast as structured `agent_activity` WS events. The server names a phase and an action; the UI owns localized labels and timing. No MCP contract change, no second deck state, no in-app model | A generation session/presence protocol; agents sending display strings; polling `/api/state`; a fake always-on "AI is thinking" state | Existing agents work unchanged, and the signal is truthful: it starts at the call and ends at the result. The UI holds the active phase for 1s and the done confirmation for 2.6s so instantaneous mutations are legible. After done there is no synthetic "working" tail because the agent is finished. Auto-follow runs only after a successful `upsert_slide`, and only when the user has been idle for 1.8s and is not in a form or the storyboard |
| 2026-10-24 | **The copilot is a server-owned agent loop with a browser-owned thread.** Keys, provider transports, attachments, system prompt and tool execution live in `slyde-server`; the UI owns visible chat state and consumes typed POST SSE. Copilot tools call `agent::run`, the same path as `/api/agent` | Browser calling providers directly; one SDK per vendor; a second copilot-only deck state; MCP as the copilot transport | Keys never reach the browser, every provider difference is isolated to one adapter layer, and the copilot cannot bypass validation, undo/redo, autosave or activity events. The MCP contract stays untouched and external agents keep working |
| 2026-10-24 | **Seven providers use the OpenAI-compatible Chat Completions wire; Anthropic uses a native Messages adapter.** Gemini uses its native `?key=` model list and the OpenAI-compatible chat base | Official per-vendor SDKs; a multi-provider abstraction crate; raw HTTP for every vendor separately | One streaming/tool-call implementation covers OpenAI, xAI, Groq, OpenRouter, DeepSeek, NVIDIA and Gemini. Anthropic is the only protocol that genuinely differs, so it gets a focused adapter instead of forcing an incompatible shape |
| 2026-10-24 | **Attachments are decoded and extracted server-side** with explicit bounds (5 MB images, 12 MB documents, 4/6 per message). Images become base64 image parts; PDF/DOCX/PPTX/text become bounded plain text | Sending raw files to providers; browser-side extraction; storing attachments in the deck | The model gets a predictable, safe shape, the browser stays thin, and no attachment ever enters the `.slyde` file or the deck state. Bounds keep a malicious or accidental file from blocking the stdio/HTTP loop |
| 2026-10-24 | **Presenter includes dual-screen Speaker Console Mode (`?speaker=1` or `S`)** in the same `present.html` surface | Separate `/speaker` route with a second WebSocket protocol | Reuses existing client, event dispatch, and sandbox origins. Speaker mode shows wall clock, stopwatch timer, current slide, next slide preview, and scalable speaker notes without requiring a separate server endpoint |
| 2026-10-24 | **Storyboard drag-and-drop uses native HTML5 Drag and Drop events** driving `move_slide` | Heavy drag-and-drop JS library | Zero bundle footprint, works seamlessly with roving tabindex accessibility, and dispatches directly to the validated `move_slide` backend command |
| 2026-10-24 | **Inspector separates Slide and Deck controls via subtabs with visual swatches**, keeping all legacy form IDs mounted | Modal dialogues or separate pages | Prevents vertical scroll fatigue while maintaining 100% backward compatibility with automated test selectors (`#theme-select`, `#export-pdf`, `#delete-slide`, `#notes`) |
| 2026-10-24 | **Copilot streaming surfaces `ChatEvent::Thinking` for reasoning models (DeepSeek, etc.)**, with reasoning fallback on empty content turns | Dropping reasoning deltas; treating reasoning as content | Reasoning models (like DeepSeek Reasoner/Flash) stream `reasoning_content` which takes 20-40s before tools/content emit. Dropping it causes perceived freezes and false "empty response" 502 errors when tools are absent. Streaming `thinking` gives user live feedback in under 300ms, and falling back to reasoning prevents empty-response failures |
| 2026-10-25 | **Math, code and diagram renderers are vendored into `assets/vendor/` and served from the sandbox origin**, loaded lazily per slide | CDN scripts; bundling Mermaid 12 with esbuild/vite; server-side rendering to SVG at save time | A slide document runs with `connect-src 'none'`, so nothing can be fetched at runtime and a CDN is impossible. Mermaid 12 is ESM-only with code-split chunks and would force a bundler; 11.12.0 still publishes a self-contained `dist/mermaid.min.js`. KaTeX is synchronous and prints correctly; highlight.js is small and deterministic. Lazy loading means decks without technical blocks pay nothing |
| 2026-10-25 | **Element handles are persisted in `Slide::code`** (`slyde_core::ensure_element_ids`) instead of being generated in the DOM | Browser-generated ids; CSS-path selectors; parsing HTML with a DOM crate | `mutate_element` is a string edit over the stored code, so an id that only exists in the browser cannot be found. Injecting `data-id` on write and on load makes the canvas, the MCP tools and the AI agree, and gives the model something to target. The injector is idempotent and skips comments plus `style`/`script` bodies |

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

### In-app copilot transport — DECIDED 2026-10-24 (direct `reqwest`, no vendor SDKs)

Candidates evaluated:

| Option | Fit with the Rust-first server | Key safety | Streaming/tool control | Added footprint | Verdict |
|---|---|---|---|---|---|
| **`reqwest` 0.12 + rustls, hand-parsed SSE** | Native, one client for every provider | Keys stay server-side | Full control of request shape, tool loop and cancellation | Moderate; already needed for MCP HTTP, rustls avoids OpenSSL | **Chosen** |
| Official SDK per vendor (OpenAI, Anthropic, Google, xAI, Groq…) | Fragmented, overlapping HTTP stacks, uneven streaming APIs | Same | Tied to each SDK's abstraction | High and volatile | Rejected: seven moving targets for one thin wire |
| One multi-provider crate / gateway | Convenient surface | Same | Often hides usage/stream quirks | Unknown maturity and license risk | Rejected: the two wire shapes are small enough to own |
| Browser calls providers directly | No Rust transport | **Unacceptable**: keys in the page | Browser owns streaming and tools | Minimal | Rejected on security and because deck mutation must go through the server |

Why direct HTTP wins: the OpenAI-compatible shape covers seven of eight
providers, and Anthropic's Messages protocol is a small, well-documented
exception. A single `reqwest::Client` gives per-request timeouts, connection
reuse and an abortable stream (dropping the response when the SSE receiver
closes). It keeps the public contract typed and testable without adopting an SDK
whose API may drift. Risks: vendor-specific tool quirks (NVIDIA/Gemini) must be
verified against real keys; a provider can change its SSE shape, so transport
tests pin the known fragmentations.

Attachments: `pdf-extract` 0.12 (MIT, pure Rust, pulls `lopdf`) for PDF text;
the existing `zip` 2.6.1 with `deflate` disabled on write but enabled for reads
to open DOCX/PPTX containers; no OCR and no image analysis beyond sending the
image to a vision model. `base64` 0.22 is the standard, well-maintained codec.

### PPTX export — OPEN, not yet decided

- Candidates: `ppt-rs` 0.2.37 (Oct 2026, ~100k downloads, active) — most promising; `pptx` 0.1.0 (Feb 2026, single release, low adoption); `office_oxide` (multi-format, commercial-ish positioning).
- Not validated yet: none of these guarantees fidelity for our generated layouts, and PowerPoint/Keynote are the judge, not the crate's test suite.
- Plan: ship PDF first (`Page.printToPDF`), then add PPTX behind an `Exporter` trait and validate real files in PowerPoint, Keynote and Google Slides before promising it.

### PNG bundling — DECIDED 2026-10-24 (`zip` 2.6.1, store-only)

Candidates evaluated:

| Option | Correctness risk | Added footprint | Verdict |
|---|---|---|---|
| `zip` 2.6.1 with `default-features = false` | Low — battle-tested CRC-32 and central directory | ~4 small crates (`crc32fast`, `indexmap`, `hashbrown`, `equivalent`); no codec pulled in | **Chosen** |
| Hand-rolled store-only ZIP writer | Real: CRC table, offsets, ZIP64 edges and a format nobody wants to maintain | 0 crates | Rejected: savings are not worth a generated-file bug |
| Browser `CompressionStream` + JS ZIP | Duplicates packaging outside the pipeline and needs N fetches | 0 Rust crates | Rejected: splits the export contract across two languages |
| Multiple PNG downloads | Chrome blocks N automatic downloads after the first | 0 | Rejected: unreliable UX |

`zip` 7.x exists but needs a newer Rust than the workspace MSRV (1.85); Cargo
resolves 2.6.1 automatically. Stored mode is deliberate: PNG is already
compressed, so deflate would add CPU and latency for no size win. The crate is
MIT and the license is compatible with a commercial product.

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
- **Export must never hold the core lock.** Snapshot title, aspect ratio and slide ids under a short read lock, release it, then let Chrome take seconds per slide. Holding the lock freezes the editor, autosave and MCP writes for the whole export.
- Each export job stages files in a unique directory under `app_data/exports/` (timestamp + pid + atomic counter) and every Chrome process gets its own profile directory. Sharing either one makes two simultaneous exports fail with a profile lock or overwrite each other's PNGs. Chrome commands use `kill_on_drop`, so an export timeout cannot orphan the process and keep the profile locked.
- PNG export spawns one headless Chrome per slide (~1–2 s each). Acceptable for decks of this size; if it becomes a bottleneck, reuse one Chrome via CDP `Page.captureScreenshot` rather than parallelising processes (which would fight for CPU and memory).
- The UI triggers downloads with a `blob:` URL and a `download` attribute; the E2E suite uses CDP `Page.setDownloadBehavior` with a temp directory and waits for the final (renamed) file. Chrome never exposes a "download finished" DOM event, so polling the file is the reliable signal.
- Export progress travels over the existing WebSocket as `export_progress` events, not as a streaming HTTP response: the final HTTP response still carries the file, so the browser can use a normal blob download and one request per export.
- **`slyde-export` must stay transport-agnostic** (no `slyde-server` dependency): `slyde-server` depends on it for `/api/export`, while the CLI lives in `slyde-app` and MCP builds its own URLs. Inverting this creates a crate cycle.
- **A CSS custom property that is referenced but not defined invalidates the whole declaration.** The stage scanline used `var(--ease-in-out)` for a while without the token existing, so `animation` was dropped silently and the sweep never ran. Tokens used by motion live in `tokens.css`; verify computed styles, not just class names.
- Headless Chrome can report `prefers-reduced-motion: reduce` even when the app is expected to animate. Visual checks of motion must call CDP `Emulation.setEmulatedMedia` with `no-preference`; otherwise the reduced-motion branch correctly disables the sweep and the screenshot looks static.
- The reduced-motion blanket rule must set `animation-iteration-count: 1 !important`, not only a tiny duration. Otherwise an `infinite` ambient animation (such as the chip shimmer) runs an enormous number of cycles per second and burns CPU while claiming to respect the preference.
- Keep the AI chip and live announcer outside the storyboard/inspector/stage containers that `state_changed` re-renders. If they live inside a re-rendered subtree, a fast state update can replace the node mid-animation and lose the activity state.
- The UI follows an agent-written slide only on the `done` event, never on the active event. Following active would load the old slide version, then reload it on `state_changed` (double iframe load and a visible flash). Following done also guarantees the target slide exists and is the final version.
- **Anthropic requires all `tool_result` blocks for one assistant turn in a single user message.** The first implementation emitted one user message per parallel tool result, which the API rejects. `anthropic_messages` now groups consecutive `Turn::Tool` entries before pushing.
- OpenAI streams tool calls in fragments: `id`/`name` may arrive once, `arguments` arrives as JSON string chunks, and parallel calls are interleaved by `index`. Accumulate by index and parse the full arguments only at `finish_reason`, never per chunk.
- Provider model lists are not chat lists. OpenAI and Gemini return embeddings, TTS, image and moderation models; normalizing must filter by known non-chat id families and shape, then dedupe and sort. The Free flag is explicit (`pricing.prompt/completion == 0` or a `:free` id), never inferred from provider tier.
- Gemini model discovery uses the native `GET /v1beta/models?key=…`; its chat uses the OpenAI-compatible base with a bearer token. Mixing those two auth styles fails in opposite directions.
- API keys are validated before persistence and must be non-empty ASCII graphic characters (max 1024). This prevents header injection through a pasted newline and rejects accidental prose before spending a network call.
- The copilot SSE stream is a POST, so native `EventSource` cannot be used. `api.js` parses `data:` frames from `ReadableStream` and ignores keep-alives; the server sends `Cache-Control: no-cache`, `X-Accel-Buffering: no` and keep-alive comments.
- The chat thread must stay `aria-live="off"` and announce only completion/error through a separate polite region. Token-by-token announcements make a screen reader unusable.
- Tool statuses are localized from tool names; provider error text is localized from stable codes. Never render model- or provider-supplied strings in the thread.
- Stop sends an `AbortController` abort from the browser; the server notices the closed SSE receiver on the next delta and returns `Cancelled`, dropping the upstream response. There is no separate cancel endpoint.
- Copilot E2E tests mock `/api/copilot/config`, `/key` and `/chat` with CDP `Fetch.fulfillRequest`. This covers streaming, tool rows and focus without a real key, and avoids any vendor network dependency. The fake SSE body is delivered as one chunk, so the parser must handle multiple events per read.
- The composer textarea autosizes by setting `height: 0`, reading `scrollHeight`, then adding 2px; without the extra pixels Chrome shows scrollbar arrow buttons on an exact-fit placeholder. The scrollbar chrome is also themed to hide the arrow buttons.
- The copilot is a desktop-panel surface: below 1100px the right panel (and the empty-state "Ask the copilot" CTA) is hidden. Do not focus a composer the user cannot see.
- **An author `display` on a class beats the user-agent `[hidden] { display: none }`.** The speaker console sets `display: flex`, so `hidden="true"` kept painting it over the whole presenter: an opaque panel with a ticking clock and empty previews, while the JS (which trusts `.hidden`) skipped rendering them. `present.css` now declares a global `[hidden] { display: none !important; }`; never reintroduce a `display` rule for a toggleable panel without it.
- **Never clip the topbar.** `.topbar { overflow: hidden }` cut off the export and present dropdowns, which are absolutely positioned below the bar: the items were laid out but invisible and unclickable (`document.elementFromPoint` returned the panel underneath). Dropdown menus must be able to overflow their trigger bar.
- **A `postMessage` into a sandboxed, cross-origin slide iframe is dropped until that document registers its listener.** The presenter now gates step delivery on the child's `ready` handshake (with `load` as the fallback); posting earlier left every animated target at `opacity: 0`, i.e. a black slide with only the HUD visible. Also: the click that opens the overlay is registered on `document` mid-bubble, so it must be ignored (`overlay.contains(target)`) or it silently eats the first animation step.
- **A clickable iframe steals the keyboard.** The first click on the presenter slide moved focus into the sandboxed document, after which the parent got no `keydown` at all: arrows, Space, Home/End, B, S, F and Esc all went dead (and the click itself never reached the overlay). The presenter frame is now non-interactive (`pointer-events: none`, `tabindex="-1"`) and the slide runtime forwards keystrokes to the parent as a fallback, validated with `event.source === frame.contentWindow`.
- **Chrome swallows Escape while in fullscreen** (it exits fullscreen instead of dispatching the key), so an Esc-only handler never fires. Treat an unexpected `fullscreenchange` to `null` as the presenter's exit gesture, and set a flag before a deliberate exit (F / HUD toggle) so going windowed does not end the presentation.
- **Never patch HTML by cutting at the first `</`.** `mutate_element` used to do that and turned `<h1 id="t">A <em>B</em></h1>` into `<h1 id="t">New</em></h1>`. Core now balances same-name tags (`find_element_span`), escapes `text` (markup goes through `html`), merges `style` into an existing attribute and returns `ElementHasNoContent` for void elements.
- **The model prompt must never describe renderers we do not have.** The copilot was told to emit Mermaid, LaTeX and highlighted code, while the sandbox renders HTML/CSS/SVG only, so decks shipped raw `graph TD;…` text. When a rendering capability lands, change the prompt in the same commit.
- **`SlideElement` is currently metadata only.** `Slide.elements` round-trips through `.slyde` and the tools can read/mutate it, but nothing populates or renders it: the canvas is one HTML blob per slide. Do not assume element insertion exists when wiring the canvas UI.
- **Select code blocks by `pre`, never `pre > code`.** Matching both processed the same block twice: the second pass rewrote the line wrappers the first one produced and collapsed the block into a single line.
- **Async renderers need a virtual-time budget in export.** Chrome prints after `load`; KaTeX, highlight.js and Mermaid finish later. `--virtual-time-budget=15000` on both the PDF and PNG commands is what keeps a diagram from printing empty. The sandbox also exposes `data-slyde-ready="1"` for anything that can wait explicitly.
- **The sandbox CSP now trusts its own origin** (`script-src`/`style-src` include `'self'`) so `/vendor` can load; `connect-src` is still `'none'` and must stay that way. `img-src` accepts `'self' data: blob:`.
- **Never rewrite UTF-8 files with PowerShell `Get-Content | Set-Content`** (PS 5.1 reads as ANSI and double-encodes): it turned `práctica` into `prÃ¡ctica` inside a probe. Use the editor tools or `[IO.File]::ReadAllText/WriteAllText` with `UTF8Encoding($false)`.

## Design System Reference

- Canonical spec: `docs/design-system.md`. Visual reference: `docs/reference/slydecode-concept.html` (mockup only, not the product UI).
- Core tokens: `--bg`/`--surface`/`--ink` neutrals per mode, `--brand` `#7C3AED` (dark: `#9B6BFF`), `--accent` `#FF6B4A` (dark: `#FF8265`).
- Signature gradient: `linear-gradient(135deg in oklch, #7C3AED, #FF6B4A)`. Decorative only: logo, AI chip, sweep, rings, display type ≥24px. Never behind small text (`#FF6B4A` on white is ~2.8:1).
- Primary buttons use `--gradient-action` (`#6D28D9 → #7C3AED`, white text ≥5.7:1); coral appears only in hover glow.
- Type: Geist Sans (bundled variable woff2; fallback Inter), mono only in Developer Mode. Base 15px, `display` 56, sentence case, no all-caps labels.
- Accessibility baseline: `--ink-subtle` was darkened to `#6F6B73` (light) / `#908D95` (dark) to clear AA on both panel surfaces; the E2E suite measures six text/token pairs per mode and fails under 4.5:1.
- Motion: 120/180/260/420/700ms with `--ease-out: cubic-bezier(.16,1,.3,1)` and `--ease-in-out: cubic-bezier(.65,0,.35,1)`; transform/opacity only; exits at 0.7×; reduced-motion degrades to opacity.
- AI activity timing: active phase holds ≥1000ms, done confirmation holds 2600ms, errors hold 4200ms; the chip enters in 180ms, the canvas sheen fades in over 200ms and the 700ms sweep runs only for `upsert_slide` on the visible slide. The `thinking` phase is emitted by the copilot's explicit SSE channel on request start and between tool rounds; external MCP agents never emit it.
- The single signature moment: generation sweep + one anchored micro-label + 40ms staggered reveal. No spinner for generation, ever. The implemented inferred-activity sweep is a 16% soft warm band led by a bright coral edge, not a bare 1px line, so it reads on dark slides.
- Copilot surface: 352px right panel with accessible **Inspector** / **Copilot** tabs; thread is `aria-live="off"` with a separate polite announcer; provider sheet covers the panel only; model picker is searchable with a `Free` toggle; errors are localized from stable codes. Active panel state persists in `localStorage`; model selection persists separately (`slyde.copilot.selection`).

## Superseded Decisions

- Three-file memory system (`MEMORY.md` + `ESTADO.md` + `TASK.md`), superseded on 2026-10-24: three files meant three reads per task, duplicated update work and allowed contradictions between state and tracker. Replaced by `MEMORY.md` + `STATE.md`.
- PDF export through the CDP `Page.printToPDF` call, superseded on 2026-10-24 by the Chrome CLI (`--print-to-pdf`): the CLI needs no WebSocket client or protocol version, and `@page size` already yields the exact page size. CDP stays as the fallback if per-page control is needed.
- "Lightweight native rendering engine" as an embedded component (Wry/Ultralight/Servo), superseded on 2026-10-24 by Chrome as the only rendering surface; the shell is now a swappable launcher.