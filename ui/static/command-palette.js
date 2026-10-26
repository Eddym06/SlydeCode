import { trapFocus } from "./focus-trap.js";
// Command palette (⌘K / Ctrl+K).
//
// One entry point for every action in the studio. It is intentionally dumb:
// the caller supplies the actions, so the palette never imports deck state and
// stays testable on its own.

export function mountCommandPalette({ root, t, buildActions }) {
  let isOpen = false;
  let actions = [];
  let visible = [];
  let cursor = 0;
  let previousFocus = null;

  root.innerHTML = `
    <div class="palette-backdrop" data-palette-close></div>
    <div class="palette-panel" role="dialog" aria-modal="true" aria-labelledby="palette-label">
      <div class="palette-search">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"
             stroke-linecap="round" aria-hidden="true">
          <circle cx="11" cy="11" r="6.5"/><path d="m16 16 4 4"/>
        </svg>
        <input id="palette-input" class="palette-input" type="text" role="combobox"
               aria-expanded="true" aria-controls="palette-list" aria-autocomplete="list"
               autocomplete="off" spellcheck="false" aria-labelledby="palette-label" />
        <kbd>esc</kbd>
      </div>
      <p class="sr-only" id="palette-label" data-i18n="palette.title">Commands</p>
      <div class="palette-list" id="palette-list" role="listbox"
           aria-labelledby="palette-label"></div>
      <p class="palette-empty" id="palette-empty" hidden data-i18n="palette.empty">
        No commands match.
      </p>
    </div>`;

  const input = root.querySelector("#palette-input");
  const list = root.querySelector("#palette-list");
  const empty = root.querySelector("#palette-empty");

  function label(action) {
    return typeof action.label === "function" ? action.label() : action.label;
  }

  function hint(action) {
    return typeof action.hint === "function" ? action.hint() : action.hint || "";
  }

  function filter() {
    const query = input.value.trim().toLowerCase();
    visible = actions.filter((action) => {
      if (action.when && !action.when()) return false;
      if (!query) return true;
      const haystack = `${label(action)} ${hint(action)} ${action.group || ""}`.toLowerCase();
      return haystack.includes(query);
    });
    cursor = Math.min(cursor, Math.max(0, visible.length - 1));
    render();
  }

  function render() {
    list.innerHTML = "";
    empty.hidden = visible.length > 0;
    let lastGroup = null;
    visible.forEach((action, index) => {
      const group = action.group || "";
      if (group && group !== lastGroup) {
        lastGroup = group;
        const heading = document.createElement("p");
        heading.className = "palette-group";
        heading.textContent = group;
        list.append(heading);
      }
      const item = document.createElement("button");
      item.type = "button";
      item.className = "palette-item";
      item.id = `palette-item-${index}`;
      item.setAttribute("role", "option");
      item.setAttribute("aria-selected", String(index === cursor));
      item.dataset.index = String(index);
      item.innerHTML = `<span class="palette-item-label">${escapeHtml(label(action))}</span>
        <span class="palette-item-hint">${escapeHtml(hint(action))}</span>`;
      if (index === cursor) item.classList.add("is-active");
      list.append(item);
    });
    const active = list.querySelector(".is-active");
    if (active) active.scrollIntoView({ block: "nearest" });
    input.setAttribute("aria-activedescendant", `palette-item-${cursor}`);
  }

  function move(delta) {
    if (!visible.length) return;
    cursor = (cursor + delta + visible.length) % visible.length;
    render();
  }

  function run(index) {
    const action = visible[index];
    if (!action) return;
    close();
    try {
      action.run();
    } catch (error) {
      console.error("palette action failed", error);
    }
  }

  let releaseTrap = null;

  function open(scope = {}) {
    if (isOpen) return;
    previousFocus = document.activeElement;
    actions = buildActions(scope) || [];
    cursor = 0;
    input.value = "";
    isOpen = true;
    root.hidden = false;
    releaseTrap = trapFocus(root);
    document.documentElement.dataset.palette = "open";
    filter();
    input.focus();
  }

  function close() {
    if (!isOpen) return;
    isOpen = false;
    root.hidden = true;
    if (releaseTrap) releaseTrap();
    releaseTrap = null;
    delete document.documentElement.dataset.palette;
    if (previousFocus && typeof previousFocus.focus === "function") {
      previousFocus.focus();
    }
    previousFocus = null;
  }

  root.addEventListener("click", (event) => {
    if (event.target.closest("[data-palette-close]")) {
      close();
      return;
    }
    const item = event.target.closest(".palette-item");
    if (item) run(Number(item.dataset.index));
  });

  root.addEventListener("mousemove", (event) => {
    const item = event.target.closest(".palette-item");
    if (!item) return;
    const index = Number(item.dataset.index);
    if (index === cursor) return;
    cursor = index;
    render();
  });

  input.addEventListener("input", filter);

  input.addEventListener("keydown", (event) => {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      move(1);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      move(-1);
    } else if (event.key === "Enter") {
      event.preventDefault();
      run(cursor);
    } else if (event.key === "Escape") {
      event.preventDefault();
      close();
    }
  });

  // Global shortcut: ⌘K / Ctrl+K. Registered here so the palette owns it.
  document.addEventListener("keydown", (event) => {
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
      event.preventDefault();
      if (isOpen) close();
      else open();
    } else if (event.key === "Escape" && isOpen) {
      event.preventDefault();
      close();
    }
  });

  return {
    open,
    close,
    isOpen: () => isOpen,
  };
}

function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}
