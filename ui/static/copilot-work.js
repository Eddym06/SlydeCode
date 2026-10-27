// What the copilot is doing while the user waits.
//
// Reasoning models can spend a minute or two before the first slide exists. A
// bare spinner reads as "stuck", so the thread shows a small progress card that
// names the real phase, counts the seconds, and shows the model's latest line of
// thought. Everything here is derived from events the server already sends; no
// timer pretends progress that has not happened.

/** Phases in the order a turn normally goes through them. */
export const PHASES = ["waiting", "thinking", "writing", "applying", "replying"];

/**
 * How many slides the user asked for, when they said so ("6 slides", "8 diapositivas").
 * Returns null when no clear number is present.
 */
export function expectedSlides(text) {
  const match = /(\d{1,2})\s*(?:slides?|diapositivas?|láminas?|laminas?|páginas?|paginas?)\b/i.exec(text || "");
  if (!match) return null;
  const n = Number(match[1]);
  return n >= 2 && n <= 40 ? n : null;
}

/** `m:ss` for the elapsed time. */
export function formatElapsed(ms) {
  const total = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}

/** The latest sentence of the reasoning, short enough for one line. */
export function lastThought(thinking, limit = 92) {
  const tail = String(thinking || "").slice(-400);
  const parts = tail
    .split(/(?<=[.!?:])\s+|\n+/)
    .map((part) => part.replace(/[*_`#>]+/g, "").trim())
    .filter((part) => part.length > 8);
  const last = parts[parts.length - 1] || "";
  if (last.length <= limit) return last;
  const cut = last.slice(0, limit - 1);
  return `${cut.slice(0, Math.max(cut.lastIndexOf(" "), 40))}…`;
}

/**
 * The tail of the reasoning as plain, wrapped text: what the model is working on
 * right now. The card shows it a few lines tall and lets it roll upward as it grows.
 */
export function recentThinking(thinking, limit = 360) {
  const text = String(thinking || "")
    .replace(/[*_`#>]+/g, "")
    .replace(/\s+/g, " ")
    .trim();
  if (text.length <= limit) return text;
  const tail = text.slice(-limit);
  // Start on a word boundary so the first visible line is not cut mid-word.
  const space = tail.indexOf(" ");
  return `…${(space >= 0 && space < 40 ? tail.slice(space + 1) : tail).trimStart()}`;
}

/**
 * The phase of a streaming assistant message.
 * @param {{text?: string, thinking?: string, tools?: {name: string, status: string, progress?: {items: number, chars: number}}[]}} message
 */
export function workPhase(message) {
  const tools = message.tools || [];
  const writing = tools.find((tool) => tool.status === "writing" || (tool.status === "running" && tool.progress));
  const running = tools.find((tool) => tool.status === "running");
  if (message.text && message.text.trim() && !running) return { phase: "replying" };
  if (writing && writing.progress && writing.status === "writing") {
    return { phase: "writing", slide: Math.max(1, writing.progress.items), chars: writing.progress.chars };
  }
  if (running) return { phase: "applying", tool: running.name };
  if (message.thinking && message.thinking.trim()) {
    return { phase: "thinking", words: message.thinking.trim().split(/\s+/).length };
  }
  return { phase: "waiting" };
}

/**
 * Segments for the progress track. With a known target each slide is a segment;
 * without one a single bar shimmers and the count tells the story.
 */
export function trackModel({ expected, written, current }) {
  if (!expected) return { segments: 0, done: written, current: false };
  const segments = Math.min(expected, 12);
  const ratio = segments / expected;
  return {
    segments,
    done: Math.min(segments, Math.round(written * ratio)),
    current: Boolean(current) && written < expected,
  };
}
