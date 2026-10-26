// A small source editor for a slide: line numbers, syntax colors and a debounced
// save. It is a transparent <textarea> laid over a highlighted <pre> with the
// same metrics, so the browser keeps handling caret, selection, IME and undo.

const escapeText = (text) =>
  text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

const span = (kind, text) => `<span class="tok-${kind}">${escapeText(text)}</span>`;

/** Colors the declarations inside a <style> block. */
function highlightCss(css) {
  let out = "";
  let index = 0;
  const pattern = /(\/\*[\s\S]*?\*\/)|([^{}]+)(\{)|([\w-]+)(\s*:)|(["'][^"']*["'])|(#[0-9a-fA-F]{3,8}\b|-?\d*\.?\d+(?:px|rem|em|%|ms|s|vh|vw|deg)?\b)|([{}])/g;
  for (let m = pattern.exec(css); m; m = pattern.exec(css)) {
    out += escapeText(css.slice(index, m.index));
    index = pattern.lastIndex;
    if (m[1]) out += span("comment", m[1]);
    else if (m[3]) out += span("selector", m[2]) + span("punct", m[3]);
    else if (m[4]) out += span("prop", m[4]) + span("punct", m[5]);
    else if (m[6]) out += span("string", m[6]);
    else if (m[7]) out += span("number", m[7]);
    else out += span("punct", m[8]);
  }
  return out + escapeText(css.slice(index));
}

/** Highlights slide markup. Every character of `source` ends up escaped. */
export function highlight(source) {
  let out = "";
  let i = 0;
  const n = source.length;
  while (i < n) {
    if (source.startsWith("<!--", i)) {
      const end = source.indexOf("-->", i + 4);
      const stop = end === -1 ? n : end + 3;
      out += span("comment", source.slice(i, stop));
      i = stop;
      continue;
    }
    if (source[i] !== "<" || !/[a-zA-Z/!]/.test(source[i + 1] || "")) {
      const next = source.indexOf("<", i + 1);
      const stop = next === -1 ? n : next;
      out += escapeText(source.slice(i, stop));
      i = stop;
      continue;
    }
    // A tag: scan to its closing ">" while respecting quoted attribute values.
    let j = i + 1;
    let quote = "";
    while (j < n) {
      const ch = source[j];
      if (quote) {
        if (ch === quote) quote = "";
      } else if (ch === '"' || ch === "'") quote = ch;
      else if (ch === ">") break;
      j += 1;
    }
    const tag = source.slice(i, Math.min(j + 1, n));
    const head = /^<\/?[\w:-]*/.exec(tag)[0];
    let rest = tag.slice(head.length);
    let tagHtml = span("punct", head.slice(0, head.startsWith("</") ? 2 : 1)) +
      span("tag", head.replace(/^<\/?/, ""));
    rest = rest.replace(/([\w:-]+)(=)?("[^"]*"|'[^']*'|[^\s>]+)?|(\/?>)|(\s+)/g, (match, name, eq, value, close, space) => {
      if (space) return space;
      if (close) return span("punct", close);
      let html = span("attr", name);
      if (eq) html += span("punct", eq);
      if (value) html += span("string", value);
      return html;
    });
    out += tagHtml + rest;
    i = Math.min(j + 1, n);

    const raw = /^<(style|script)\b/i.exec(tag);
    if (raw && !tag.endsWith("/>")) {
      const close = source.toLowerCase().indexOf(`</${raw[1].toLowerCase()}`, i);
      const stop = close === -1 ? n : close;
      const body = source.slice(i, stop);
      out += raw[1].toLowerCase() === "style" ? highlightCss(body) : escapeText(body);
      i = stop;
    }
  }
  return out;
}

/**
 * @param {object} options
 * @param {string} options.value initial source
 * @param {string} options.label accessible name of the textarea
 * @param {(value: string) => void} options.onSave debounced, called after typing settles
 * @param {() => void} [options.onBlur]
 * @param {number} [options.delay] save debounce in ms
 */
export function createCodeEditor({ value, label, onSave, onBlur, delay = 600 }) {
  const el = document.createElement("div");
  el.className = "code-editor";
  el.innerHTML = `
    <div class="code-body">
      <div class="code-gutter" aria-hidden="true"></div>
      <div class="code-stack">
        <pre class="code-highlight" aria-hidden="true"></pre>
        <textarea class="code-input" spellcheck="false" autocomplete="off" autocapitalize="off"
                  wrap="off"></textarea>
      </div>
    </div>
    <div class="code-status" role="status" aria-live="polite"></div>`;
  const input = el.querySelector(".code-input");
  const pre = el.querySelector(".code-highlight");
  const gutter = el.querySelector(".code-gutter");
  const status = el.querySelector(".code-status");
  input.setAttribute("aria-label", label);

  let timer = null;
  let dirty = false;

  function paint() {
    const text = input.value;
    // The trailing newline keeps the last empty line the same height as the textarea.
    pre.innerHTML = `${highlight(text)}\n`;
    const lines = text.split("\n").length;
    if (gutter.childElementCount !== lines) {
      gutter.textContent = Array.from({ length: lines }, (_, k) => k + 1).join("\n");
    }
    status.dataset.dirty = String(dirty);
    status.textContent = `${lines} · ${text.length}`;
  }

  function syncScroll() {
    pre.scrollTop = input.scrollTop;
    pre.scrollLeft = input.scrollLeft;
    gutter.scrollTop = input.scrollTop;
  }

  input.value = value;
  paint();

  input.addEventListener("input", () => {
    dirty = true;
    paint();
    if (timer) window.clearTimeout(timer);
    timer = window.setTimeout(() => {
      timer = null;
      dirty = false;
      paint();
      onSave(input.value);
    }, delay);
  });
  input.addEventListener("scroll", syncScroll);
  input.addEventListener("keydown", (event) => {
    // Tab indents instead of leaving the editor; Escape leaves it, so a keyboard
    // user is never trapped inside the textarea.
    if (event.key === "Tab" && !event.shiftKey && !event.ctrlKey && !event.metaKey) {
      event.preventDefault();
      const { selectionStart: start, selectionEnd: end } = input;
      input.setRangeText("  ", start, end, "end");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    }
  });
  input.addEventListener("blur", () => {
    if (onBlur) onBlur();
  });

  return {
    el,
    input,
    get dirty() {
      return dirty;
    },
    get focused() {
      return document.activeElement === input;
    },
    setValue(next) {
      if (next === input.value) return;
      input.value = next;
      paint();
    },
    selection() {
      return { start: input.selectionStart, end: input.selectionEnd, top: input.scrollTop, left: input.scrollLeft };
    },
    restore(state) {
      input.focus({ preventScroll: true });
      try {
        input.setSelectionRange(state.start, state.end);
      } catch {
        /* A detached node throws; focus alone is still useful. */
      }
      input.scrollTop = state.top;
      input.scrollLeft = state.left;
      syncScroll();
    },
  };
}
