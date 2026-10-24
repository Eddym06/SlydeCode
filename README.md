# SlydeCode

A native, AI-driven presentation studio. The Rust core owns the deck and serves
it over `127.0.0.1`; the user's Google Chrome is the only rendering surface,
both for editing and for presenting. AI agents drive the deck through an MCP
stdio server.

## Requirements

- Rust (stable). On this machine the MSVC target cannot link, so builds use
  `+stable-x86_64-pc-windows-gnu`.
- Google Chrome, Chromium or Edge (Chromium-based).
- Node.js 24+ for the UI end-to-end tests (no npm dependencies).

## Build and run

```powershell
cargo +stable-x86_64-pc-windows-gnu build --workspace
target\debug\slyde.exe app
```

`app` opens the studio in Chrome. `app --present` opens the presenter in kiosk
mode, `app --no-open` prints the session URL instead.

## Commands

| Command | Purpose |
|---|---|
| `slyde app` | Start the studio and open Chrome. |
| `slyde app --present` | Start in presenter/kiosk mode. |
| `slyde mcp` | MCP server over stdio (for Claude Desktop, Cursor, etc.). |
| `slyde export --format pdf --out deck.pdf` | Print the current deck with headless Chrome. |
| `slyde export --format png --out exports/slides` | One PNG per slide. |
| `slyde version` / `slyde help` | Version and usage. |

`export` requires a running studio instance, located through the runtime file
in the OS app-data directory. The same pipeline is available inside the studio:
the inspector's **Export** section renders a PDF, or a ZIP with one PNG per
slide, and downloads it with the deck's name.

External MCP activity is visible in the studio: a localized status chip shows
what the agent is doing, a warm scanline sweeps the slide being written, and the
stage follows the written slide when the user is idle. The signal is inferred
from the existing `/api/agent` path, so agents do not need to change.

## Tests

```powershell
cargo +stable-x86_64-pc-windows-gnu test --workspace
node --test "tests/ui/*.test.mjs"
```

The UI suite spawns the built binary with an isolated `SLYDE_DATA_DIR`, launches
headless Chrome and drives the real interface over CDP. It covers the activity
state machine, fonts, sandbox isolation, storyboard keyboard navigation, runtime
i18n, theme toggle, notes autosave, `.slyde` export, in-app PDF/PNG export with a
real download, agent activity + conditional auto-follow, sandbox slide rendering,
presenter progress and the accessibility baseline.

## Environment

| Variable | Effect |
|---|---|
| `SLYDE_DATA_DIR` | Override the app-data directory (runtime file, decks, Chrome profile). |
| `SLYDE_UI_DIR` | Serve the UI from another directory. |
| `SLYDE_ASSETS_DIR` | Serve fonts/brand assets from another directory. |
| `SLYDE_BROWSER` | Chrome/Chromium executable used for launch, export and tests. |
| `RUST_LOG` | Tracing filter (logs always go to stderr; stdout belongs to MCP). |

## Documentation

- `AGENTS.md` — engineering charter and quality bar.
- `MEMORY.md` — durable decisions, research and gotchas.
- `STATE.md` — current state, known issues and prioritized backlog.
- `docs/design-system.md` — tokens, typography, motion and accessibility.
- `docs/reference/` — visual mockups (not the product UI).
