// The elements library: shapes, icons, charts and tables you can drop on a slide
// without writing code. A popover anchored to the stage, operable with the
// keyboard (tabs, arrow keys between tiles, Escape to close).

import { CATALOG, CATEGORIES, centeredPlacement } from "./elements-catalog.js";
import { trapFocus } from "./focus-trap.js";

export function mountElementsLibrary({ trigger, t, applyI18n, insert }) {
  let popover = null;
  let category = CATEGORIES[0];
  let release = null;

  function build() {
    popover = document.createElement("div");
    popover.className = "elements-popover";
    popover.id = "elements-popover";
    popover.setAttribute("role", "dialog");
    popover.setAttribute("aria-label", t("elements.title"));
    popover.hidden = true;
    document.querySelector(".stage-wrap").append(popover);
    popover.addEventListener("click", onClick);
    popover.addEventListener("keydown", onKeydown);
  }

  function tabsMarkup() {
    return CATEGORIES.map(
      (id) =>
        `<button type="button" class="elements-tab" role="tab" data-category="${id}"
          aria-selected="${id === category}" tabindex="${id === category ? 0 : -1}">${t(`elements.category.${id}`)}</button>`
    ).join("");
  }

  function gridMarkup() {
    return CATALOG.filter((item) => item.category === category)
      .map(
        (item, index) => `
        <button type="button" class="elements-tile" data-item="${item.id}" data-category="${item.category}"
                style="--i:${index}" aria-label="${t(`elements.item.${item.category}.${item.id}`)}"
                title="${t(`elements.item.${item.category}.${item.id}`)}">
          <span class="elements-preview">${item.html}</span>
        </button>`
      )
      .join("");
  }

  function render() {
    popover.innerHTML = `
      <header class="elements-head">
        <h2>${t("elements.title")}</h2>
        <button type="button" class="icon-btn icon-btn-sm" data-close aria-label="${t("shortcuts.close")}">✕</button>
      </header>
      <div class="elements-tabs" role="tablist" aria-label="${t("elements.title")}">${tabsMarkup()}</div>
      <div class="elements-grid" role="tabpanel" data-cat="${category}">${gridMarkup()}</div>`;
    applyI18n(popover);
  }

  function open() {
    if (!popover) build();
    render();
    popover.hidden = false;
    trigger.setAttribute("aria-expanded", "true");
    release = trapFocus(popover);
    const first = popover.querySelector(".elements-tab[aria-selected='true']");
    if (first) first.focus();
  }

  function close({ restoreFocus = true } = {}) {
    if (!popover || popover.hidden) return;
    popover.hidden = true;
    trigger.setAttribute("aria-expanded", "false");
    if (release) release();
    release = null;
    if (restoreFocus) trigger.focus();
  }

  function onClick(event) {
    if (event.target.closest("[data-close]")) {
      close();
      return;
    }
    const tab = event.target.closest(".elements-tab");
    if (tab) {
      category = tab.dataset.category;
      render();
      popover.querySelector(".elements-tab[aria-selected='true']").focus();
      return;
    }
    const tile = event.target.closest(".elements-tile");
    if (tile) {
      const item = CATALOG.find((entry) => entry.id === tile.dataset.item && entry.category === tile.dataset.category);
      if (item) {
        insert(item, centeredPlacement);
        close({ restoreFocus: false });
      }
    }
  }

  function onKeydown(event) {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      close();
      return;
    }
    const tab = event.target.closest(".elements-tab");
    if (tab && (event.key === "ArrowRight" || event.key === "ArrowLeft")) {
      event.preventDefault();
      const delta = event.key === "ArrowRight" ? 1 : -1;
      category = CATEGORIES[(CATEGORIES.indexOf(category) + delta + CATEGORIES.length) % CATEGORIES.length];
      render();
      popover.querySelector(".elements-tab[aria-selected='true']").focus();
      return;
    }
    const tile = event.target.closest(".elements-tile");
    if (tile && event.key.startsWith("Arrow")) {
      const tiles = [...popover.querySelectorAll(".elements-tile")];
      const columns = Math.max(1, Math.round(popover.querySelector(".elements-grid").clientWidth / tile.offsetWidth));
      const step = { ArrowRight: 1, ArrowLeft: -1, ArrowDown: columns, ArrowUp: -columns }[event.key];
      const next = tiles[tiles.indexOf(tile) + step];
      if (next) {
        event.preventDefault();
        next.focus();
      }
    }
  }

  trigger.addEventListener("click", () => (popover && !popover.hidden ? close() : open()));
  document.addEventListener("pointerdown", (event) => {
    if (popover && !popover.hidden && !popover.contains(event.target) && !trigger.contains(event.target)) {
      close({ restoreFocus: false });
    }
  });

  return { open, close };
}
