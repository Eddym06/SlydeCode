// Keyboard shortcut sheet, opened with "?". It owns its own markup so the shell
// only has to mount it; strings come from the locale dictionaries.

import { trapFocus } from "./focus-trap.js";

const GROUPS = [
  {
    title: "shortcuts.studio",
    rows: [
      ["Ctrl/⌘ K", "shortcuts.palette"],
      ["Ctrl/⌘ Z", "shortcuts.undo"],
      ["Ctrl/⌘ ⇧ Z", "shortcuts.redo"],
      ["↑ ↓", "shortcuts.slides"],
      ["Ctrl/⌘ V", "shortcuts.paste"],
      ["?", "shortcuts.help"],
    ],
  },
  {
    title: "shortcuts.canvas",
    rows: [
      ["← ↑ → ↓", "shortcuts.nudge"],
      ["⇧ + drag", "shortcuts.resize"],
      ["Esc", "shortcuts.deselect"],
    ],
  },
  {
    title: "shortcuts.present",
    rows: [
      ["Space →", "shortcuts.next"],
      ["←", "shortcuts.prev"],
      ["B", "shortcuts.blank"],
      ["S", "shortcuts.speaker"],
      ["F", "shortcuts.fullscreen"],
    ],
  },
];

export function mountShortcuts({ t, applyI18n }) {
  const root = document.createElement("div");
  root.className = "shortcuts";
  root.hidden = true;
  root.innerHTML = `
    <div class="shortcuts-sheet" role="dialog" aria-modal="true" aria-labelledby="shortcuts-title" tabindex="-1">
      <header class="shortcuts-head">
        <h2 id="shortcuts-title" data-i18n="shortcuts.title"></h2>
        <button type="button" class="icon-btn" data-close data-i18n-aria="shortcuts.close" aria-label="Close">✕</button>
      </header>
      <div class="shortcuts-body">
        ${GROUPS.map(
          (group) => `<section>
            <h3 data-i18n="${group.title}"></h3>
            <dl>${group.rows
              .map(([keys, label]) => `<div><dt><kbd>${keys}</kbd></dt><dd data-i18n="${label}"></dd></div>`)
              .join("")}</dl>
          </section>`
        ).join("")}
      </div>
    </div>`;
  document.body.append(root);
  applyI18n(root);

  const sheet = root.querySelector(".shortcuts-sheet");
  let returnFocus = null;
  let release = null;

  function open() {
    returnFocus = document.activeElement;
    applyI18n(root);
    root.hidden = false;
    release = trapFocus(sheet);
    sheet.focus();
  }

  function close() {
    root.hidden = true;
    if (release) release();
    release = null;
    if (returnFocus && typeof returnFocus.focus === "function") returnFocus.focus();
  }

  root.addEventListener("pointerdown", (event) => {
    if (event.target === root) close();
  });
  root.querySelector("[data-close]").addEventListener("click", close);
  document.addEventListener("keydown", (event) => {
    if (!root.hidden && event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      close();
      return;
    }
    const tag = (event.target.tagName || "").toLowerCase();
    const typing =
      tag === "input" || tag === "textarea" || tag === "select" || event.target.isContentEditable;
    if (event.key === "?" && !typing && root.hidden) {
      event.preventDefault();
      open();
    }
  });

  return { open, close };
}
