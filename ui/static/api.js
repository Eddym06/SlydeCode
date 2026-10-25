// HTTP + WebSocket client for the local server. The token arrives in the URL
// once (handed over by the launcher) and is kept in sessionStorage.

export function readToken() {
  const url = new URL(window.location.href);
  const fromQuery = url.searchParams.get("t");
  if (fromQuery) {
    sessionStorage.setItem("slyde.token", fromQuery);
    url.searchParams.delete("t");
    window.history.replaceState({}, "", url.pathname + url.search + url.hash);
    return fromQuery;
  }
  return sessionStorage.getItem("slyde.token");
}

export function createClient(token) {
  // The raw query form is still needed where the browser owns the request and
  // headers are impossible: the WebSocket handshake, plain <a download> links and
  // window.open. Everything else authenticates with a bearer header, so the
  // session token stays out of URLs, history and request logs.
  const query = `t=${encodeURIComponent(token)}`;
  const authHeaders = { authorization: `Bearer ${token}` };

  async function json(path, init) {
    const response = await fetch(path, {
      ...init,
      headers: { ...authHeaders, ...((init && init.headers) || {}) },
    });
    let payload = null;
    try {
      payload = await response.json();
    } catch {
      payload = null;
    }
    if (!response.ok || !payload || payload.ok === false) {
      const message = payload && payload.error ? payload.error : `HTTP ${response.status}`;
      const error = new Error(message);
      error.code = payload && payload.code ? payload.code : "request_failed";
      throw error;
    }
    return payload;
  }

  return {
    token,

    getState() {
      return json(`/api/state`);
    },

    command(action, extra = {}) {
      return json(`/api/command`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ action, ...extra }),
      });
    },

    /** Opens a `.slyde` file uploaded from the user's disk. */
    openDeck(text) {
      return json(`/api/deck/open`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: text,
      });
    },

    async copilotConfig() {
      const payload = await json(`/api/copilot/config`);
      return payload.config;
    },

    async copilotModels(provider) {
      const payload = await json(
        `/api/copilot/models?provider=${encodeURIComponent(provider)}`
      );
      return payload.models || [];
    },

    async copilotSaveKey(provider, key) {
      const payload = await json(`/api/copilot/key`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ provider, key }),
      });
      return payload.models || [];
    },

    /** The conversation stored for one deck. */
    async copilotThread(deck) {
      const payload = await json(`/api/copilot/thread?deck=${encodeURIComponent(deck)}`);
      return payload.messages || [];
    },

    /** Replaces the conversation stored for one deck. */
    async copilotSaveThread(deck, messages) {
      return json(`/api/copilot/thread`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ deck, messages }),
      });
    },

    /** Stops a running turn on the server (the local fetch is aborted too). */
    copilotCancel(requestId) {
      return json(`/api/copilot/cancel`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ request_id: requestId }),
      });
    },

    /** Stores a pasted/uploaded image and returns its `/assets/…` URL. */
    async uploadAsset(dataUrl) {
      const payload = await json(`/api/assets`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ data: dataUrl }),
      });
      return payload.url;
    },

    // ---------- deck library ----------

    /** Every deck on disk plus the one that is currently open. */
    decks() {
      return json(`/api/decks`);
    },

    deckNew(title) {
      return json(`/api/decks/new`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(title ? { title } : {}),
      });
    },

    deckOpen(id) {
      return json(`/api/decks/open`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ id }),
      });
    },

    deckRename(id, title) {
      return json(`/api/decks/rename`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ id, title }),
      });
    },

    deckDuplicate(id) {
      return json(`/api/decks/duplicate`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ id }),
      });
    },

    deckDelete(id) {
      return json(`/api/decks/delete`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ id }),
      });
    },

    /**
     * POSTs a chat turn and consumes the SSE response. The server frames each
     * event as a single `data:` JSON line, so the parser stays small and does
     * not need an EventSource (which cannot POST or carry an abort signal).
     */
    async streamChat(payload, { signal, onEvent } = {}) {
      const response = await fetch(`/api/copilot/chat`, {
        method: "POST",
        headers: { "content-type": "application/json", ...authHeaders },
        body: JSON.stringify(payload),
        signal,
      });
      if (!response.ok) {
        let body = null;
        try {
          body = await response.json();
        } catch {
          body = null;
        }
        const error = new Error(body && body.error ? body.error : `HTTP ${response.status}`);
        error.code = body && body.code ? body.code : "copilot_failed";
        throw error;
      }
      if (!response.body) {
        const error = new Error("streaming is not available in this browser");
        error.code = "network";
        throw error;
      }

      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      let buffer = "";

      const dispatch = (block) => {
        if (!block.trim()) return;
        const data = block
          .split("\n")
          .filter((line) => line.startsWith("data:"))
          .map((line) => line.slice(5).replace(/^ /, ""))
          .join("\n");
        if (!data || data === "keep-alive") return;
        try {
          const event = JSON.parse(data);
          if (event && event.type) onEvent(event);
        } catch {
          // A provider or proxy should never send malformed frames through the
          // server, but one bad frame must not kill the rest of the stream.
        }
      };

      while (true) {
        const { value, done } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true }).replace(/\r\n/g, "\n");
        let boundary = buffer.indexOf("\n\n");
        while (boundary >= 0) {
          dispatch(buffer.slice(0, boundary));
          buffer = buffer.slice(boundary + 2);
          boundary = buffer.indexOf("\n\n");
        }
      }
      buffer += decoder.decode();
      if (buffer.trim()) dispatch(buffer);
    },

    /**
     * Renders the deck with headless Chrome and returns the finished file.
     * Progress arrives over the WebSocket as `export_progress` events.
     */
    async exportDeck(format) {
      const response = await fetch(`/api/export?format=${encodeURIComponent(format)}`, {
        method: "POST",
        headers: authHeaders,
      });
      if (!response.ok) {
        let payload = null;
        try {
          payload = await response.json();
        } catch {
          payload = null;
        }
        const error = new Error(
          payload && payload.error ? payload.error : `HTTP ${response.status}`
        );
        error.code = payload && payload.code ? payload.code : "export_failed";
        throw error;
      }
      return {
        blob: await response.blob(),
        filename: filenameFromDisposition(response.headers.get("content-disposition")),
      };
    },

    /**
     * Connects the event stream. Returns a disposer. Reconnects with backoff;
     * the server sends the full view on every (re)connect, so no replay logic.
     */
    connect({ onEvent, onStatus }) {
      let socket = null;
      let closed = false;
      let attempt = 0;
      let timer = null;

      const open = () => {
        if (closed) return;
        onStatus("connecting");
        const scheme = window.location.protocol === "https:" ? "wss" : "ws";
        socket = new WebSocket(`${scheme}://${window.location.host}/ws?${query}`);

        socket.addEventListener("open", () => {
          attempt = 0;
          onStatus("online");
        });

        socket.addEventListener("message", (event) => {
          let payload = null;
          try {
            payload = JSON.parse(event.data);
          } catch {
            return;
          }
          if (payload && payload.type && payload.type !== "pong") onEvent(payload);
        });

        socket.addEventListener("close", () => {
          if (closed) return;
          onStatus("offline");
          attempt += 1;
          const delay = Math.min(5000, 400 * 2 ** Math.min(attempt, 4));
          timer = window.setTimeout(open, delay);
        });

        socket.addEventListener("error", () => {
          if (socket) socket.close();
        });
      };

      open();

      return () => {
        closed = true;
        if (timer) window.clearTimeout(timer);
        if (socket) socket.close();
      };
    },
  };
}

/// Prefers the RFC 5987 UTF-8 name, which preserves accents in deck titles.
function filenameFromDisposition(header) {
  if (!header) return null;
  const utf8 = /filename\*=UTF-8''([^;]+)/i.exec(header);
  if (utf8) {
    try {
      return decodeURIComponent(utf8[1]);
    } catch {
      return null;
    }
  }
  const plain = /filename="([^"]+)"/i.exec(header);
  return plain ? plain[1] : null;
}
