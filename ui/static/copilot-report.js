// The report: what a provider actually said when it refused a request.
//
// The thread shows a short, localized sentence for every failure, because a raw
// provider payload is noise in a conversation. When that sentence is not enough
// ("rejected the request" tells nobody what to change), the Report button opens
// this panel: the provider, the model, the HTTP status, a plain explanation and
// the provider's own reply, formatted and ready to copy into a support ticket.

import { trapFocus } from "./focus-trap.js";

/** Pretty-prints a JSON reply and colors it; anything else is shown as it came. */
export function formatReply(text, escapeHtml) {
  const raw = String(text || "").trim();
  if (!raw) return "";
  let shown = raw;
  try {
    shown = JSON.stringify(JSON.parse(raw), null, 2);
  } catch {
    // Not JSON (a gateway page, a plain sentence): keep it verbatim.
    return escapeHtml(raw);
  }
  // Tokenize the raw text and escape each piece afterwards: escaping first would
  // turn the quotes into entities and hide the strings from the pattern.
  const token = /("(?:\\.|[^"\\])*")(\s*:)?|\b(true|false|null)\b|-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?/g;
  let out = "";
  let last = 0;
  for (const match of shown.matchAll(token)) {
    out += escapeHtml(shown.slice(last, match.index));
    const [whole, string, colon, literal] = match;
    if (string) {
      out += colon
        ? `<span class="rp-key">${escapeHtml(string)}</span>${escapeHtml(colon)}`
        : `<span class="rp-str">${escapeHtml(string)}</span>`;
    } else if (literal) {
      out += `<span class="rp-lit">${escapeHtml(whole)}</span>`;
    } else {
      out += `<span class="rp-num">${escapeHtml(whole)}</span>`;
    }
    last = match.index + whole.length;
  }
  return out + escapeHtml(shown.slice(last));
}

/** The plain-text version that goes to the clipboard. */
export function reportText(report, t) {
  const lines = [
    "SlydeCode copilot report",
    `${t("copilot.report.provider")}: ${report.provider || "-"}`,
    `${t("copilot.report.model")}: ${report.model || "-"}`,
    `${t("copilot.report.status")}: ${report.status ? `HTTP ${report.status}` : "-"}`,
    `${t("copilot.report.code")}: ${report.code || "-"}`,
    `${t("copilot.report.when")}: ${report.when || ""}`,
    "",
    report.summary || "",
    "",
    `${t("copilot.report.reply")}:`,
    report.detail || "-",
  ];
  return lines.join("\n");
}

async function writeClipboard(text) {
  if (navigator.clipboard && window.isSecureContext !== false) {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      // Fall through to the selection method below.
    }
  }
  const area = document.createElement("textarea");
  area.value = text;
  area.setAttribute("readonly", "");
  area.style.cssText = "position:fixed;left:-9999px;top:0;opacity:0";
  document.body.append(area);
  area.select();
  let ok = false;
  try {
    ok = document.execCommand("copy");
  } catch {
    ok = false;
  }
  area.remove();
  return ok;
}

/**
 * @param {object} options
 * @param {HTMLElement} options.host          the copilot panel; the report covers it
 * @param {(key: string, vars?: object) => string} options.t
 * @param {(value: string) => string} options.escapeHtml
 */
export function mountReport({ host, t, escapeHtml }) {
  let root = null;
  let release = null;
  let opener = null;
  let copyTimer = null;

  // Listened for on the document, not on the dialog: copying can move focus out of
  // it (the fallback selects a temporary field), and Escape must still close it.
  function onEscape(event) {
    if (event.key !== "Escape" || !root) return;
    event.preventDefault();
    event.stopPropagation();
    close();
  }

  function close() {
    if (!root) return;
    document.removeEventListener("keydown", onEscape, true);
    if (release) release();
    release = null;
    root.remove();
    root = null;
    clearTimeout(copyTimer);
    if (opener && document.contains(opener)) opener.focus();
    opener = null;
  }

  /**
   * @param {{provider?: string, model?: string, status?: number|null, code?: string,
   *          summary?: string, hint?: string, detail?: string, when?: string}} report
   */
  function open(report, trigger) {
    close();
    opener = trigger || document.activeElement;
    root = document.createElement("div");
    root.className = "report-layer";
    const status = report.status ? `HTTP ${report.status}` : "";
    const chips = [
      [t("copilot.report.provider"), report.provider],
      [t("copilot.report.model"), report.model],
      [t("copilot.report.status"), status],
      [t("copilot.report.code"), report.code],
    ].filter(([, value]) => value);

    root.innerHTML = `
      <div class="report-backdrop" data-close></div>
      <section class="report" role="dialog" aria-modal="true" aria-labelledby="report-title">
        <header class="report-head">
          <span class="report-icon" aria-hidden="true">
            <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3.5 2.8 19.5h18.4L12 3.5Z"/><path d="M12 10v4.2M12 17.2h.01"/></svg>
          </span>
          <div class="report-title">
            <h2 id="report-title">${escapeHtml(t("copilot.report.title"))}</h2>
            <p>${escapeHtml(report.summary || "")}</p>
          </div>
          <button type="button" class="report-close" data-close aria-label="${escapeHtml(t("copilot.report.close"))}">
            <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" aria-hidden="true"><path d="M6 6l12 12M18 6 6 18"/></svg>
          </button>
        </header>
        <dl class="report-meta">
          ${chips
            .map(
              ([label, value]) =>
                `<div><dt>${escapeHtml(label)}</dt><dd>${escapeHtml(String(value))}</dd></div>`
            )
            .join("")}
        </dl>
        ${report.hint ? `<p class="report-hint">${escapeHtml(report.hint)}</p>` : ""}
        <div class="report-code">
          <div class="report-code-head">
            <span>${escapeHtml(t("copilot.report.reply"))}</span>
            <button type="button" class="report-copy" data-copy>
              <svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="8.5" y="8.5" width="11" height="11" rx="2.5"/><path d="M5.5 15.5v-9a2 2 0 0 1 2-2h9"/></svg>
              <span>${escapeHtml(t("copilot.report.copy"))}</span>
            </button>
          </div>
          <pre tabindex="0"><code>${formatReply(report.detail, escapeHtml) || escapeHtml(t("copilot.report.empty"))}</code></pre>
        </div>
      </section>`;
    host.append(root);

    root.addEventListener("click", (event) => {
      if (event.target.closest("[data-close]")) close();
    });
    document.addEventListener("keydown", onEscape, true);
    const copy = root.querySelector("[data-copy]");
    copy.addEventListener("click", async () => {
      const ok = await writeClipboard(reportText(report, t));
      // The fallback path selects a temporary field; bring focus back to the dialog.
      if (root) copy.focus();
      const label = copy.querySelector("span");
      copy.dataset.state = ok ? "done" : "failed";
      label.textContent = t(ok ? "copilot.report.copied" : "copilot.report.copyFailed");
      clearTimeout(copyTimer);
      copyTimer = setTimeout(() => {
        if (!root) return;
        delete copy.dataset.state;
        label.textContent = t("copilot.report.copy");
      }, 1800);
    });

    release = trapFocus(root.querySelector(".report"));
    root.querySelector(".report-close").focus();
  }

  return { open, close, isOpen: () => root !== null };
}
