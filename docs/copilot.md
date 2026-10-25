# Copilot Architecture

The copilot is an in-app conversation that edits the same live deck the
inspector, presenter and MCP server use. It is deliberately split across the
existing browser/server boundary: the browser owns presentation and streaming
state, while the Rust server owns keys, provider transports, attachment
extraction, the system prompt and the tool loop.

## Why this shape

- **One deck, one mutation path.** The model never gets a private deck copy. Its
  tools call the same `agent::run` path as `/api/agent`, including validation,
  undo/redo, autosave, activity events and state broadcasts.
- **Keys never reach the browser.** The server stores them in the app-data
  directory and exposes only `configured: true/false` per provider.
- **Provider differences stay server-side.** Seven providers speak the
  OpenAI-compatible Chat Completions wire; Anthropic gets a native Messages
  adapter. The UI only knows provider ids, names, marks and model metadata.
- **Streaming is a product surface.** A POST SSE response carries typed events,
  not raw provider payloads, so the UI can localize every state and ignore
  vendor-specific noise.

## Modules

| File | Responsibility |
|---|---|
| `copilot/catalog.rs` | Provider ids, names, marks, key URLs, env keys, endpoints, model-list shape. |
| `copilot/config.rs` | `copilot.json` load/save, in-memory store for tests, owner-only Unix permissions. |
| `copilot/mod.rs` | `CopilotState`: provider view, key validation, model discovery, last provider/model. |
| `copilot/models.rs` | Normalizes model lists, removes non-chat models, derives free/vision/context metadata. |
| `copilot/attachments.rs` | Base64 decode, image/document limits, PDF/DOCX/PPTX/text extraction. |
| `copilot/prompt.rs` | Presentation-quality system prompt and per-language guidance. |
| `copilot/tools.rs` | Eight tool schemas plus OpenAI/Anthropic serialization and execution. |
| `copilot/transport.rs` | OpenAI and Anthropic SSE parsing, fragmented tool-call assembly, error mapping. |
| `copilot/chat.rs` | Request limits, history construction, the 8-round tool loop, SSE emission. |
| `copilot/routes.rs` | Guarded HTTP handlers, body limits, keep-alive SSE headers. |

The frontend counterpart is `ui/static/copilot.js` (thread, composer,
attachments, streaming lifecycle), `ui/static/copilot-model-sheet.js` (provider
and model picker) and `ui/static/provider-icons.js`.

## HTTP surface

All routes require the session token and an allowed `Origin`, like the rest of
the API.

| Method | Route | Purpose |
|---|---|---|
| `GET` | `/api/copilot/config` | Provider status, last provider/model. Never returns keys. |
| `GET` | `/api/copilot/models?provider=…` | Fetches and normalizes models for a configured provider. |
| `POST` | `/api/copilot/key` | Validates a key by listing models, then persists it. An empty key removes it. |
| `POST` | `/api/copilot/chat` | Streams a chat turn as SSE. Body limit: 32 MB. |

### SSE events

Each `data:` frame is one JSON object:

```json
{"type":"meta","provider":"deepseek","model":"deepseek-flash"}
{"type":"thinking","text":"Let me read the deck to see existing slides."}
{"type":"tool","name":"read_deck","status":"running"}
{"type":"tool","name":"read_deck","status":"done"}
{"type":"delta","text":"I'll start with a cover."}
{"type":"done"}
{"type":"error","code":"rate_limited","message":"…"}
```

- `meta` is sent before any provider call.
- `thinking` carries live reasoning tokens from reasoning models (e.g. DeepSeek or Anthropic thinking deltas), surfaced in the UI as an active thinking pill with thought preview.
- `delta` carries visible assistant text.
- `tool` status is `running`, `done` or `error`.
- `done` means the tool loop finished successfully. A client abort ends the
  response without a `done`; the UI marks the message as stopped.
- `error.code` is stable (`missing_key`, `invalid_key`, `rate_limited`,
  `network`, `upstream`, `unsupported`, `invalid_request`, `cancelled`). The UI
  owns localized copy and never displays `message` as the primary explanation.

## Tools

`read_deck`, `read_slide`, `create_presentation`, `upsert_slide`, `remove_slide`,
`move_slide`, `trigger_animation`, `set_notes`.

They mirror the MCP command surface, and every mutation runs through
`agent::run`, so an agent and the copilot are indistinguishable to the deck.
Reads are local and do not broadcast activity. The chat itself emits a
`thinking` activity phase on request start and between tool rounds, then the
shared `done`/`error` outcome; the stage chip and the panel's own live dot are
therefore consistent.

## Limits

- Chat: last turn must be `user`, max 40 messages, 80k text characters, max 8
  attachments across the request, max 8 tool rounds.
- Attachments: max 6 per message, 5 MB images, 12 MB documents, 30k extracted
  text characters per document.
- Body: 32 MB. Provider calls: 15s connect, 60s default timeout, 300s per chat
  request.
- Client history: last 24 messages; only the last two user messages carry
  attachments, capped at 6 total.

## Security and privacy

- Keys are stored in `%APPDATA%\SlydeCode\copilot.json` (or the equivalent
  app-data path), written atomically, with owner-only permissions on Unix.
  Environment variables are the fallback when no key is stored.
- The key is validated against the provider before it is persisted; malformed or
  absurdly long keys are rejected locally before any network call.
- Provider error bodies are reduced to a short public message; raw bodies and
  headers stay in the server logs.
- Slide code remains untrusted and keeps rendering in the cross-origin sandbox.
  Copilot-generated code gets no extra privileges.
- The copilot mutates the deck only through the tool surface, which is the same
  `agent::run` path the MCP bridge uses; `mutate_element` calls are scoped to a
  `slide_id` so a generated handle cannot edit the wrong slide.
- A network failure keeps its cause chain (DNS, TLS, refused) in the public
  message; a provider rejection keeps its first line. Neither path echoes keys.
- The browser test suite mocks provider HTTP at the CDP boundary, so tests never
  require a real key or vendor network access.

## Testing

- Rust: provider catalog and normalization, free-model detection, config
  persistence, key rejection, attachment extraction/limits, tool schemas,
  Anthropic/OpenAI stream assembly (including parallel tool results).
- UI E2E: tab switch, connect/key form, eight provider marks, model search,
  Free filter, model selection, focus restoration, EN/ES rerender, attachment
  chip/remove, SSE text + tool rows + completion announcement, accessibility.

## Known gaps

- There is no conversation persistence across reloads; the thread lives in the
  page. Keys and model selection persist.
- Stop aborts the HTTP stream and drops the provider response; the server does
  not send a cancellation frame.
- The copilot is desktop-only while the right panel is hidden below 1100px.
- Token usage/cost is not estimated or displayed.
