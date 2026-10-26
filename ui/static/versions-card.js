// The "Versions" card of the Deck tab: named checkpoints of the deck.
//
// It owns its own state (the list lives on the server, per deck) and re-renders
// only itself, so saving a version never rebuilds the rest of the panel.

/** "3 min ago" in the interface language. */
export function relativeTime(thenMs, nowMs, locale) {
  const seconds = Math.round((thenMs - nowMs) / 1000);
  const units = [
    ["day", 86400],
    ["hour", 3600],
    ["minute", 60],
  ];
  const formatter = new Intl.RelativeTimeFormat(locale, { numeric: "auto" });
  for (const [unit, size] of units) {
    if (Math.abs(seconds) >= size) return formatter.format(Math.round(seconds / size), unit);
  }
  return formatter.format(0, "second");
}

export function createVersionsCard({ client, t, escapeHtml, notify, lang, applyI18n, confirmRestore }) {
  let list = [];
  let host = null;
  let busy = false;
  let loaded = false;
  let lastRefresh = 0;

  function markup() {
    const rows = list
      .map(
        (version) => `
        <li class="version-row" data-version="${escapeHtml(version.id)}">
          <span class="version-dot${version.auto ? " is-auto" : ""}" aria-hidden="true"></span>
          <span class="version-copy">
            <span class="version-name">${escapeHtml(version.name)}</span>
            <span class="version-meta">${escapeHtml(relativeTime(version.created_at_ms, Date.now(), lang()))} · ${escapeHtml(
              t("versions.slides", { n: version.slides })
            )}${version.auto ? ` · ${escapeHtml(t("versions.auto"))}` : ""}</span>
          </span>
          <button type="button" class="version-btn" data-restore="${escapeHtml(version.id)}">${escapeHtml(t("versions.restore"))}</button>
          <button type="button" class="version-btn version-delete" data-delete="${escapeHtml(version.id)}"
                  aria-label="${escapeHtml(t("versions.delete"))}" title="${escapeHtml(t("versions.delete"))}">✕</button>
        </li>`
      )
      .join("");
    return `
      <form class="version-form" data-version-form>
        <input class="version-input" name="name" maxlength="80" autocomplete="off"
               placeholder="${escapeHtml(t("versions.placeholder"))}"
               aria-label="${escapeHtml(t("versions.placeholder"))}">
        <button type="submit" class="version-save">${escapeHtml(t("versions.save"))}</button>
      </form>
      ${
        rows
          ? `<ul class="version-list">${rows}</ul>`
          : `<p class="hint">${escapeHtml(loaded ? t("versions.empty") : t("versions.loading"))}</p>`
      }`;
  }

  function paint() {
    if (!host || !host.isConnected) return;
    const typed = host.querySelector(".version-input");
    const keep = typed ? { value: typed.value, focus: document.activeElement === typed } : null;
    host.innerHTML = markup();
    if (keep) {
      const input = host.querySelector(".version-input");
      input.value = keep.value;
      if (keep.focus) input.focus();
    }
    applyI18n(host);
  }

  async function run(action) {
    if (busy) return;
    busy = true;
    try {
      await action();
    } catch (error) {
      notify(error && error.message ? error.message : t("toast.error"));
    } finally {
      busy = false;
      paint();
    }
  }

  async function refresh() {
    lastRefresh = Date.now();
    try {
      list = await client.versions();
    } catch {
      list = [];
    }
    loaded = true;
    paint();
  }

  function bind() {
    host.addEventListener("submit", (event) => {
      event.preventDefault();
      const input = host.querySelector(".version-input");
      const name = input.value.trim();
      run(async () => {
        list = await client.saveVersion(name || t("versions.defaultName"));
        input.value = "";
        notify(t("versions.saved"));
      });
    });
    host.addEventListener("click", (event) => {
      const restore = event.target.closest("[data-restore]");
      const remove = event.target.closest("[data-delete]");
      if (restore) {
        const id = restore.dataset.restore;
        const version = list.find((item) => item.id === id);
        if (confirmRestore && !confirmRestore(version)) return;
        run(async () => {
          const payload = await client.restoreVersion(id);
          list = payload.versions || list;
          notify(t("versions.restored"));
        });
      } else if (remove) {
        const id = remove.dataset.delete;
        run(async () => {
          list = await client.deleteVersion(id);
        });
      }
    });
  }

  // The copilot saves a checkpoint before it changes the deck.
  document.addEventListener("slyde:versions-changed", () => {
    if (host && host.isConnected) refresh();
    else lastRefresh = 0;
  });

  return {
    /** Attach to a freshly rendered container (the panel rebuilds it). */
    mount(container) {
      const changed = host !== container;
      host = container;
      if (changed) bind();
      paint();
      // The panel re-renders on every deck change; the list only changes when a
      // version is saved or restored, so do not ask the server each time.
      if (!loaded || Date.now() - lastRefresh > 15000) refresh();
    },
    refresh,
  };
}
