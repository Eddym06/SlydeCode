# `ui/src` — reserved layout

The frontend that ships today is plain ES modules in `ui/static/`:

| File | Responsibility |
|---|---|
| `api.js` | HTTP/WS client, token handling, SSE stream reader |
| `app.js` | Studio shell: topbar, stage frame, chips, palette actions and boot glue |
| `presenter-overlay.js` | In-studio presenter: reveal, speaker console and timer, HUD, blackout, fullscreen, laser/pen. Owns its state; talks to the shell through getters and `onClosed` |
| `agent-activity-view.js` | AI activity chip, live announcement, stage sweep and the "is the user busy" rule |
| `copilot.js` | In-app copilot panel (providers, thread, reasoning drawer) |
| `copilot-model-sheet.js` | Provider/model picker |
| `command-palette.js` | `⌘K` / `Ctrl+K` palette |
| `agent-activity.js` | Activity state machine (pure, unit-tested) |
| `i18n.js` | EN/ES dictionaries plus the `t()` layer |
| `present.js`, `teleprompter.js` | Standalone presenter and pacing |
| `tokens.css`, `present.css` | Design tokens and the standalone presenter surface |
| `topbar.css`, `workspace.css`, `inspector.css`, `widgets.css`, `canvas-library.css` | Studio shell surfaces. `index.html` links them in this order and the cascade depends on it |
| `copilot.css`, `copilot-reasoning.css`, `copilot-sheet.css` | Copilot thread, reasoning drawer, provider and model sheet |

These directories are the target for the split described in `STATE.md`:
`components/` (one component per file), `features/` (storyboard, inspector,
presenter, ai-generation), `locales/en|es` and `styles/`. They stay empty until
that refactor lands, so nothing here should be imported yet.

When moving a file out of `ui/static/`, update the `<script src>` list in
`ui/index.html` / `ui/present.html` and the E2E suite in the same change: the UI
has no bundler, so a path that is wrong is a blank page.
