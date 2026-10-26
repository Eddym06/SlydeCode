// Topbar dropdown menus (File, View).
//
// A trigger carries `data-menu="<menu id>"`. The menu closes on Escape, on an
// outside click, when focus leaves it, and after choosing an item (unless the
// item says `data-keep-open`). Arrow keys walk the items, and focus returns to
// the trigger on close, as the WAI-ARIA menu button pattern asks.

const ITEM = '[role="menuitem"], [role="menuitemcheckbox"]';

export function mountMenus(root = document) {
  const triggers = [...root.querySelectorAll("[data-menu]")];
  const entries = triggers.map((trigger) => ({
    trigger,
    menu: root.getElementById
      ? root.getElementById(trigger.dataset.menu)
      : document.getElementById(trigger.dataset.menu),
  }));

  function setOpen(entry, open, { restoreFocus = false } = {}) {
    if (!entry.menu) return;
    entry.menu.hidden = !open;
    entry.trigger.setAttribute("aria-expanded", String(open));
    entry.trigger.parentElement.classList.toggle("is-open", open);
    if (!open && restoreFocus) entry.trigger.focus();
  }

  function closeAll(except = null) {
    entries.forEach((entry) => {
      if (entry !== except) setOpen(entry, false);
    });
  }

  entries.forEach((entry) => {
    const { trigger, menu } = entry;
    if (!menu) return;

    trigger.addEventListener("click", (event) => {
      event.stopPropagation();
      const open = menu.hidden;
      closeAll(entry);
      setOpen(entry, open);
      if (open && event.detail === 0) menu.querySelector(ITEM)?.focus();
    });

    trigger.addEventListener("keydown", (event) => {
      if (event.key === "ArrowDown" && menu.hidden) {
        event.preventDefault();
        closeAll(entry);
        setOpen(entry, true);
        menu.querySelector(ITEM)?.focus();
      }
    });

    menu.addEventListener("click", (event) => {
      const item = event.target.closest(ITEM);
      if (item && !item.hasAttribute("data-keep-open")) setOpen(entry, false);
    });

    menu.addEventListener("keydown", (event) => {
      const items = [...menu.querySelectorAll(ITEM)];
      const index = items.indexOf(document.activeElement);
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        setOpen(entry, false, { restoreFocus: true });
      } else if (event.key === "ArrowDown") {
        event.preventDefault();
        items[(index + 1) % items.length]?.focus();
      } else if (event.key === "ArrowUp") {
        event.preventDefault();
        items[(index - 1 + items.length) % items.length]?.focus();
      } else if (event.key === "Home") {
        event.preventDefault();
        items[0]?.focus();
      } else if (event.key === "End") {
        event.preventDefault();
        items[items.length - 1]?.focus();
      } else if (event.key === "Tab") {
        setOpen(entry, false);
      }
    });
  });

  document.addEventListener("click", (event) => {
    entries.forEach((entry) => {
      if (entry.menu && !entry.menu.hidden && !entry.trigger.parentElement.contains(event.target)) {
        setOpen(entry, false);
      }
    });
  });

  return { closeAll };
}
