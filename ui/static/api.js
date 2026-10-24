// HTTP + WebSocket client for the local server. The token arrives in the URL
// once (handed over by the launcher) and is kept in sessionStorage.

export function readToken() {
  const url = new URL(window.location.href);
  const fromQuery = url.searchParams.get("t");
  if (fromQuery) {
    sessionStorage.setItem("slyde.token", fromQuery);
    url.searchParams.delete("t");
    window.history.replaceState({}, "", url.pathname + url.search);
    return fromQuery;
  }
  return sessionStorage.getItem("slyde.token");
}

export function createClient(token) {
  const query = `t=${encodeURIComponent(token)}`;

  async function json(path, init) {
    const response = await fetch(path, init);
    let payload = null;
    try {
      payload = await response.json();
    } catch {
      payload = null;
    }
    if (!response.ok || !payload || payload.ok === false) {
      const message = payload && payload.error ? payload.error : `HTTP ${response.status}`;
      throw new Error(message);
    }
    return payload;
  }

  return {
    token,

    getState() {
      return json(`/api/state?${query}`);
    },

    command(action, extra = {}) {
      return json(`/api/command?${query}`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ action, ...extra }),
      });
    },

    /** Opens a `.slyde` file uploaded from the user's disk. */
    openDeck(text) {
      return json(`/api/deck/open?${query}`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: text,
      });
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
