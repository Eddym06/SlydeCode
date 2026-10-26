// Deck library sheet.
//
// Before this existed the studio had one deck: starting another one replaced it.
// The sheet lists every deck on disk, and every action goes through the server's
// library API, which owns the files. The studio only applies the read model that
// comes back, so there is no second source of truth here.
//
// It is a sheet, not a route: opening, renaming or deleting a deck never reloads
// the page, and Escape/backdrop click close it like every other surface.

const RELATIVE_BUCKETS = [
  { limitMs: 60_000, key: "library.time.now" },
  { limitMs: 3_600_000, key: "library.time.minutes", per: 60_000 },
  { limitMs: 86_400_000, key: "library.time.hours", per: 3_600_000 },
  { limitMs: 7 * 86_400_000, key: "library.time.days", per: 86_400_000 },
];

function relativeLabel(t, updatedAtMs, nowMs) {
  const delta = Math.max(0, nowMs - (updatedAtMs || 0));
  for (const bucket of RELATIVE_BUCKETS) {
    if (delta < bucket.limitMs) {
      return bucket.per
        ? t(bucket.key, { n: Math.max(1, Math.round(delta / bucket.per)) })
        : t(bucket.key);
    }
  }
  return t("library.time.older");
}

function escapeHtml(value) {
  return String(value == null ? "" : value)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

export function mountDeckLibrary({ root, client, t, notify, onApplied }) {
  if (!root) throw new Error("deck library needs a root element");

  let decks = [];
  let active = null;
  let query = "";
  let editingId = null;
  let confirmDeleteId = null;
  let open = false;
  let lastFocus = null;
  // Covers the server has rendered, as id -> the `updated_at_ms` they match. A
  // deck that changed has a new key, so its old cover is never shown.
  const covers = new Map();
  let coverRun = 0;
  // Filled in by the consumer: the sheet renders translated markup itself, so it
  // needs the same `apply(root)` pass the rest of the UI uses.
  let applyI18n = () => {};

  root.innerHTML = `
    <div class="library-scrim" data-library-action="close"></div>
    <section class="library-sheet" role="dialog" aria-modal="true" aria-labelledby="library-title">
      <header class="library-head">
        <div>
          <h2 id="library-title" class="library-title" data-i18n="library.title"></h2>
          <p class="library-subtitle" data-i18n="library.subtitle"></p>
        </div>
        <button type="button" class="icon-btn" data-library-action="close" data-i18n-aria="library.close">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">
            <path d="M18 6 6 18M6 6l12 12"/>
          </svg>
        </button>
      </header>
      <div class="library-toolbar">
        <button type="button" class="btn btn-primary" data-library-action="new">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true" width="16" height="16">
            <path d="M12 5v14M5 12h14"/>
          </svg>
          <span data-i18n="library.new"></span>
        </button>
        <label class="library-search">
          <span class="sr-only" data-i18n="library.search"></span>
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true" width="16" height="16">
            <circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/>
          </svg>
          <input type="search" id="library-search" autocomplete="off" data-i18n-placeholder="library.search" />
        </label>
      </div>
      <ul class="library-list" id="library-list"></ul>
      <p class="library-empty" id="library-empty" hidden data-i18n="library.empty"></p>
      <p class="library-error" id="library-error" role="alert" hidden></p>
    </section>`;

  const els = {
    list: root.querySelector("#library-list"),
    empty: root.querySelector("#library-empty"),
    error: root.querySelector("#library-error"),
    search: root.querySelector("#library-search"),
    sheet: root.querySelector(".library-sheet"),
  };

  function showError(message) {
    if (!message) {
      els.error.hidden = true;
      els.error.textContent = "";
      return;
    }
    els.error.hidden = false;
    els.error.textContent = message;
  }

  function metaLine(deck) {
    const slides =
      deck.slides === 1
        ? t("library.slidesOne", { n: 1 })
        : t("library.slidesMany", { n: deck.slides });
    const when = relativeLabel(t, deck.updated_at_ms, Date.now());
    const theme = t(`theme.${deck.theme_id}`);
    return `${slides} · ${when} · ${escapeHtml(theme)}`;
  }

  function coverMarkup(deck) {
    const ready = covers.get(deck.id) === deck.updated_at_ms;
    const inner = ready
      ? `<img alt="" decoding="async" src="${escapeHtml(
          client.deckThumbnailUrl(deck.id, deck.updated_at_ms)
        )}" />`
      : `<span class="deck-cover-initial" aria-hidden="true">${escapeHtml(
          (deck.title.trim()[0] || "·").toUpperCase()
        )}</span>`;
    return `<span class="deck-cover${ready ? " is-ready" : ""}" data-cover-for="${escapeHtml(deck.id)}" data-ratio="${escapeHtml(deck.aspect_ratio)}">${inner}</span>`;
  }

  function rowMarkup(deck) {
    const isActive = deck.id === active;
    if (editingId === deck.id) {
      return `
        <li class="deck-row is-editing" data-deck-id="${escapeHtml(deck.id)}">
          <form class="deck-rename" data-library-form="rename">
            <input type="text" class="deck-rename-input" id="deck-rename-input"
                   value="${escapeHtml(deck.title)}" data-i18n-aria="library.rename" maxlength="120" />
            <button type="submit" class="btn btn-primary btn-sm" data-i18n="library.renameSave"></button>
            <button type="button" class="btn btn-ghost btn-sm" data-library-action="cancel-rename" data-i18n="library.cancel"></button>
          </form>
        </li>`;
    }
    const confirming = confirmDeleteId === deck.id;
    return `
      <li class="deck-row${isActive ? " is-active" : ""}" data-deck-id="${escapeHtml(deck.id)}">
        <button type="button" class="deck-open" data-library-action="open" data-deck-id="${escapeHtml(deck.id)}">
          ${coverMarkup(deck)}
          <span class="deck-text">
            <span class="deck-row-title">${escapeHtml(deck.title)}</span>
            <span class="deck-row-meta">${metaLine(deck)}</span>
            ${
              isActive
                ? `<span class="deck-badge" data-i18n="library.active"></span>`
                : ""
            }
          </span>
        </button>
        <div class="deck-actions">
          ${
            confirming
              ? `<span class="deck-confirm" data-i18n="library.deleteConfirmShort"></span>
                 <button type="button" class="btn btn-danger btn-sm" data-library-action="delete-confirm" data-deck-id="${escapeHtml(deck.id)}" data-i18n="library.delete"></button>
                 <button type="button" class="btn btn-ghost btn-sm" data-library-action="cancel-delete" data-i18n="library.cancel"></button>`
              : `<button type="button" class="btn btn-ghost btn-sm" data-library-action="rename" data-deck-id="${escapeHtml(deck.id)}" data-i18n="library.rename"></button>
                 <button type="button" class="btn btn-ghost btn-sm" data-library-action="duplicate" data-deck-id="${escapeHtml(deck.id)}" data-i18n="library.duplicate"></button>
                 <button type="button" class="btn btn-ghost btn-sm deck-delete" data-library-action="delete" data-deck-id="${escapeHtml(deck.id)}" data-i18n="library.delete"></button>`
          }
        </div>
      </li>`;
  }

  function visibleDecks() {
    const needle = query.trim().toLowerCase();
    if (!needle) return decks;
    return decks.filter((deck) => deck.title.toLowerCase().includes(needle));
  }

  function render() {
    const rows = visibleDecks();
    els.list.innerHTML = rows.map(rowMarkup).join("");
    els.empty.hidden = rows.length > 0;
    if (decks.length === 0) {
      els.empty.textContent = t("library.empty");
    } else if (rows.length === 0) {
      els.empty.textContent = t("library.noMatches");
      els.empty.hidden = false;
    }
    applyI18n();
  }


  // Asks for the covers of the decks the sheet shows. The server renders the
  // missing ones in one go; the images are then swapped in place, so a cover
  // arriving never rebuilds the list under a rename field or a pending delete.
  async function loadCovers() {
    if (!open || typeof client.deckThumbnails !== "function") return;
    const run = (coverRun += 1);
    const wanted = visibleDecks()
      .filter((deck) => deck.slides > 0 && covers.get(deck.id) !== deck.updated_at_ms)
      .slice(0, 24);
    if (wanted.length === 0) return;
    let payload;
    try {
      payload = await client.deckThumbnails(wanted.map((deck) => deck.id));
    } catch {
      return; // covers are decoration: the list works without them
    }
    if (run !== coverRun || !open || !payload || !Array.isArray(payload.ready)) return;
    for (const id of payload.ready) {
      const deck = decks.find((candidate) => candidate.id === id);
      if (!deck) continue;
      covers.set(id, deck.updated_at_ms);
      const slot = els.list.querySelector(`[data-cover-for="${CSS.escape(id)}"]`);
      if (slot) {
        slot.outerHTML = coverMarkup(deck);
      }
    }
  }

  function setData(payload) {
    decks = Array.isArray(payload && payload.decks) ? payload.decks : [];
    active = payload && typeof payload.active === "string" ? payload.active : null;
    for (const id of [...covers.keys()]) {
      if (!decks.some((deck) => deck.id === id)) covers.delete(id);
    }
    render();
    loadCovers();
  }

  async function call(action) {
    try {
      showError(null);
      await action();
    } catch (error) {
      showError(error && error.message ? error.message : t("library.error"));
    }
  }

  function applied(payload) {
    setData(payload);
    if (payload && payload.view && typeof onApplied === "function") {
      onApplied(payload.view);
    }
  }

  const handlers = {
    close: () => close(),
    new: () =>
      call(async () => {
        applied(await client.deckNew());
        notify(t("library.created"));
      }),
    open: (button) =>
      call(async () => {
        const id = button.dataset.deckId;
        if (id === active) {
          close();
          return;
        }
        applied(await client.deckOpen(id));
        close();
      }),
    rename: (button) => {
      editingId = button.dataset.deckId;
      render();
      const input = els.list.querySelector("#deck-rename-input");
      if (input) {
        input.focus();
        input.select();
      }
    },
    "cancel-rename": () => {
      editingId = null;
      render();
    },
    duplicate: (button) =>
      call(async () => {
        applied(await client.deckDuplicate(button.dataset.deckId));
        notify(t("library.duplicated"));
      }),
    delete: (button) => {
      confirmDeleteId = button.dataset.deckId;
      render();
    },
    "cancel-delete": () => {
      confirmDeleteId = null;
      render();
    },
    "delete-confirm": (button) =>
      call(async () => {
        confirmDeleteId = null;
        applied(await client.deckDelete(button.dataset.deckId));
        notify(t("library.deleted"));
      }),
  };

  root.addEventListener("click", (event) => {
    const target = event.target.closest("[data-library-action]");
    if (!target) return;
    const action = handlers[target.dataset.libraryAction];
    if (action) {
      event.preventDefault();
      action(target);
    }
  });

  root.addEventListener("submit", (event) => {
    const form = event.target.closest('[data-library-form="rename"]');
    if (!form) return;
    event.preventDefault();
    const row = form.closest("[data-deck-id]");
    const input = form.querySelector("#deck-rename-input");
    if (!row || !input) return;
    const id = row.dataset.deckId;
    const title = input.value.trim();
    if (!title) {
      input.focus();
      return;
    }
    call(async () => {
      editingId = null;
      applied(await client.deckRename(id, title));
      notify(t("library.renamed"));
    });
  });

  els.search.addEventListener("input", () => {
    query = els.search.value;
    render();
    loadCovers();
  });

  function onKeydown(event) {
    if (!open) return;
    if (event.key === "Escape") {
      event.preventDefault();
      if (editingId || confirmDeleteId) {
        editingId = null;
        confirmDeleteId = null;
        render();
        return;
      }
      close();
    }
  }

  document.addEventListener("keydown", onKeydown);

  function openSheet() {
    if (open) return;
    open = true;
    lastFocus = document.activeElement;
    root.hidden = false;
    query = "";
    editingId = null;
    confirmDeleteId = null;
    if (els.search) els.search.value = "";
    call(async () => {
      setData(await client.decks());
    });
    els.search?.focus({ preventScroll: true });
  }

  function close() {
    if (!open) return;
    open = false;
    editingId = null;
    confirmDeleteId = null;
    root.hidden = true;
    showError(null);
    if (lastFocus && typeof lastFocus.focus === "function") {
      lastFocus.focus({ preventScroll: true });
    }
    lastFocus = null;
  }

  return {
    open: openSheet,
    close,
    /** Re-reads the library, e.g. after the deck title changed in the topbar. */
    refresh: () =>
      call(async () => {
        setData(await client.decks());
      }),
    setI18n: (apply) => {
      applyI18n = apply;
      render();
    },
    isOpen: () => open,
  };
}

/** Relative timestamp helper exported for tests. */
export const __test__ = { relativeLabel };
