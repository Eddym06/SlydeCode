// Keeps keyboard focus inside a dialog while it is open.
//
// Tab and Shift+Tab wrap around the dialog's focusable controls, and the
// returned function releases the trap. Dialogs already restore focus to the
// element that opened them; this only prevents Tab from escaping behind them.

const FOCUSABLE = [
  "a[href]",
  "button:not([disabled])",
  "input:not([disabled]):not([type='hidden'])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  "[tabindex]:not([tabindex='-1'])",
].join(",");

/** Visible, enabled controls inside `container`, in DOM order. */
export function focusables(container) {
  return [...container.querySelectorAll(FOCUSABLE)].filter(
    (element) => !element.hidden && !element.closest("[hidden]") && element.getClientRects().length > 0
  );
}

/** Where Tab should land when it would leave the list; null means "let it go". */
export function wrapTarget(items, active, backwards) {
  if (!items.length) return null;
  const first = items[0];
  const last = items[items.length - 1];
  if (backwards && (active === first || !items.includes(active))) return last;
  if (!backwards && (active === last || !items.includes(active))) return first;
  return null;
}

export function trapFocus(container) {
  function onKeydown(event) {
    if (event.key !== "Tab") return;
    const target = wrapTarget(focusables(container), document.activeElement, event.shiftKey);
    if (target) {
      event.preventDefault();
      target.focus();
    }
  }
  document.addEventListener("keydown", onKeydown, true);
  return () => document.removeEventListener("keydown", onKeydown, true);
}
