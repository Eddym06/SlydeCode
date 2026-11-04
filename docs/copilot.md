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
- **Keys never reach the browser.** The server keeps them in the OS credential store (an
  owner-only file where there is none) and exposes only `configured: true/false` per provider.
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
| `copilot/config.rs` | `copilot.json` load/save (settings, plus keys when no credential store is available), key migration, owner-only permissions. |
| `copilot/vault.rs` | `Vault` trait, the OS credential store (Windows/macOS) and an in-memory vault for tests. |
| `copilot/mod.rs` | `CopilotState`: provider view, key validation, model discovery, last provider/model. |
| `copilot/models.rs` | Normalizes model lists, removes non-chat models, derives free/vision/context metadata. |
| `copilot/attachments.rs` | Base64 decode, image/document limits, PDF/DOCX/PPTX/text extraction. |
| `copilot/prompt.rs` | Presentation-quality system prompt and per-language guidance. |
| `copilot/tools.rs` | Eighteen tool definitions (sixteen advertised per turn) plus OpenAI/Anthropic serialization and execution. |
| `copilot/imagegen.rs` | Provider image generation (OpenAI, Gemini, xAI, OpenRouter), request builders, response parsers and balance probes. |
| `copilot/image_service.rs` | Background image jobs: capability cache, budget reservations, rate limit, parallel workers, slide wait tickets and placement. |
| `copilot/stock.rs` | Unsplash/Pexels/Vecteezy search and attribution for decks with no usable image generator. |
| `copilot/transport.rs` | OpenAI and Anthropic SSE parsing, fragmented tool-call assembly, error mapping. |
| `copilot/chat.rs` | Request limits, history construction, the 40-round tool loop, SSE emission. |
| `copilot/routes.rs` | Guarded HTTP handlers, body limits, keep-alive SSE headers. |

The frontend counterpart is `ui/static/copilot.js` (thread, composer,
attachments, streaming lifecycle), `ui/static/copilot-model-sheet.js` (provider
and model picker) and `ui/static/provider-icons.js`.

## HTTP surface

All routes require the session token and an allowed `Origin`, like the rest of
the API.

| Method | Route | Purpose |
|---|---|---|
| `GET` | `/api/copilot/config` | Provider status, last chat provider/model and last image provider/model. Never returns keys. |
| `GET` | `/api/copilot/models?provider=…&kind=image` | Fetches and normalizes chat models, or the provider's picture models when `kind=image`. OpenRouter returns its full `/images/models` catalog (61 routes at the time of writing), not just the text-capable subset. Token prices are normalized to USD per 1M tokens where the vendor publishes them. |
| `POST` | `/api/copilot/key` | Validates a key by listing models, then persists it. An empty key removes it. |
| `POST` | `/api/copilot/image-model` | Remembers which provider/model draws pictures, without touching the chat selection. |
| `POST` | `/api/copilot/stock-key` | Saves or removes an Unsplash, Pexels or Vecteezy key for the stock fallback. |
| `POST` | `/api/copilot/chat` | Streams a chat turn as SSE. Body limit: 32 MB. |
| `POST` | `/api/copilot/cancel` | Stops the turn for a `request_id`; the upstream response and queued tool rounds are dropped. |
| `POST` | `/api/copilot/revert` | One-step undo of the last copilot turn, valid only while the deck is exactly as that turn left it. |
| `GET` / `POST` | `/api/copilot/thread` | Loads or saves the deck's conversation (text only, bounded). |
| `POST` | `/api/copilot/fetch-url` | Reads a page as text or an image as an attachment for the composer. |
| `POST` | `/api/images/generate` | Generates one picture with a connected provider, stores it in the asset store and returns its `/assets/…` URL. |

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
- `progress` carries the live draft markup and the slide count while the model writes.
- `usage` reports provider calls plus estimated input tokens for the turn.
- `error` carries a typed `code`, a localized sentence and, when the provider refused,
  its short reply with the key redacted.
- `done` means the tool loop finished successfully. A client abort ends the
  response without a `done`; the UI marks the message as stopped.
- `error.code` is stable (`missing_key`, `invalid_key`, `rate_limited`,
  `network`, `upstream`, `unsupported`, `invalid_request`, `cancelled`). The UI
  owns localized copy and never displays `message` as the primary explanation.

## Tools

`read_deck`, `read_slide`, `create_presentation`, `upsert_slide`, `remove_slide`,
`move_slide`, `trigger_animation`, `set_notes`, `set_theme`, `set_title`,
`mutate_element`, `get_selected_element`, `write_slides`, `build_deck`.

They mirror the MCP command surface, and every mutation runs through
`agent::run`, so an agent and the copilot are indistinguishable to the deck.
Reads are local and do not broadcast activity. The chat itself emits a
`thinking` activity phase on request start and between tool rounds, then the
shared `done`/`error` outcome; the stage chip and the panel's own live dot are
therefore consistent.

## Limits

- Chat: last turn must be `user`, max 40 messages, 80k text characters, max 8
  attachments across the request, max 40 tool rounds (the turn also stops early after
  three identical tool calls, and reports a settled `tool_limit` instead of a fake
  provider error).
- Attachments: max 6 per message, 5 MB images, 12 MB documents, 30k extracted
  text characters per document.
- Body: 32 MB. Provider calls: 15s connect, 60s default timeout, 300s per chat
  request.
- Client history: last 24 messages; only the last two user messages carry
  attachments, capped at 6 total.

## Images

Every path ends in the deck's asset store instead of leaving a remote URL in the
slide, because the sandbox refuses to fetch the network (`connect-src 'none'`):

- `insert_image(slide_id, src, alt, x?, y?, w?, h?)` accepts an `https:` link, a
  `data:image/…` URL or an existing `/assets/…` path. Remote bytes are downloaded
  with `webfetch` (private and loopback addresses blocked, 5 MiB ceiling, 12 s
  timeout, four redirects at most) before the deck lock is taken, so a slow host
  never freezes autosave. Geometry defaults to a centred picture at 60% of the
  stage and is clamped to the canvas.
- `request_image(image_id, prompt, alt, slide_id|slide_index, x?, y?, w?, h?)`
  is what the in-app copilot uses. It only names a unique id, a visual prompt and
  a target; the image service owns the provider, model, internal spend guard,
  parallelism and rate limit. The call returns a queued ticket immediately, so
  the text stream keeps writing while the picture is generated. Results are
  buffered in the asset store and placed on the requested slide; if the slide
  does not exist yet, the job waits (up to 120 s) instead of failing or landing on
  the wrong slide. At most 10 pictures start per minute, 3 generate in parallel
  and a turn may request at most 10. The prompt is only told that an image model
  is available: no balance, price or predicted image count is sent to the text
  model. Cancelling the textual turn also cancels its queued and in-flight image
  jobs.
- `search_images(query, kind?, count?, orientation?)` is advertised when no image
  generator is usable and a stock service is configured. It searches Unsplash and
  Pexels for photos and Vecteezy for vectors, returning direct URLs, dimensions
  and attribution; the model then places the chosen result with `insert_image`.
  Keys can be saved from the copilot sheet's **Stock image keys** screen, next to
  the provider keys, or supplied through `UNSPLASH_ACCESS_KEY`,
  `PEXELS_API_KEY`, and `VECTEEZY_API_KEY` + `VECTEEZY_ACCOUNT_ID`.
- `generate_image(prompt, provider?, slide_id?, …)` remains the direct,
  synchronous tool for MCP and integrations. It calls the selected image model:
  OpenAI (`gpt-image-1` by default, or another image model from its list), Gemini
  (`gemini-2.5-flash-image` and siblings), xAI `grok-2-image-1212`, or any of
  OpenRouter's `/images/models` routes. The key never reaches the model or the
  browser; the result is decoded server-side, stored like an upload and, when
  `slide_id` is present, inserted with the same geometry rules. A failed placement
  still keeps the generated asset, so a retry never pays for a second picture.

The model sheet's **Model for: Image** view lists every image model the provider
publishes. OpenRouter is read from `/api/v1/images/models` (61 routes when this
was written), then token prices are merged from `/api/v1/models` for the subset
that publishes them. The sheet shows a green down arrow for input and a red up
arrow for output, both in USD per 1M tokens; models that bill per picture say so.
Selecting a model also asks its `/endpoints` pricing once, so the capability
prompt reserves the right amount instead of always using the catalog estimate.

The copilot system prompt gets a dedicated image block only while a configured
image model is healthy. The block simply says that an image model is available
and how to call `request_image`; it does not expose a model name, balance, price
or predicted image count. It also tells the model to decide when a picture helps
instead of waiting for an explicit request, and to use a cover and one or two key
visual moments rather than an image on every slide. After a generation failure
the block disappears for 90 s, the stock fallback takes over when configured, and
the chat shows the localized error when the textual turn closes.

MCP exposes `insert_image` and `generate_image`. `POST /api/images/generate` is
the shared direct endpoint and reports provider failures with the same
`CopilotError` mapping as chat.

### Layout review

After `build_deck`, `write_slides` or `upsert_slide`, the server opens only the
changed slides in a real browser and measures them. The model receives one
review, and only hard failures: `broken`, `empty`, `overflow`, `clipped`,
`overlap` and `contrast`. Cosmetic audit warnings such as small labels, tight
margins or dense data are not returned as work, because they are frequently
intentional and would trigger a whole-deck rewrite. Local edits such as
`mutate_element` or `insert_image` do not start the review automatically, and a
slide whose background image is still queued is not reported as `empty`.

## Security and privacy

- Keys are stored in the OS credential store (Windows Credential Manager, macOS
  Keychain) under the service `SlydeCode`. Where there is none (Linux) or when it
  refuses a write, they fall back to `copilot.json` in the app-data directory,
  written atomically with owner-only permissions. A key an older version left in
  that file is moved into the credential store on the next start. A custom
  `SLYDE_DATA_DIR` never uses the credential store, so tests and second profiles
  stay isolated. Environment variables are the fallback when no key is stored.
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

- The thread survives a reload: it is stored per deck next to the keys
  (`GET`/`POST /api/copilot/thread`, bounded to 40 messages and 100 decks). Only the
  text is kept; tool rows and reasoning from the previous session are not replayed.
- Stop aborts the HTTP stream and `POST /api/copilot/cancel` stops the turn
  server-side, dropping the upstream response, any tool rounds still queued and
  every image job that turn had enqueued or in flight.
- The copilot is desktop-only while the right panel is hidden below 1100px.
- Token usage/cost is not estimated or displayed.
