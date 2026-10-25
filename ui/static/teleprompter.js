// Speaker pacing: how long the notes should take to say, and whether the
// presenter is on it. Pure functions so both presenter surfaces share one rule.

/** Average speaking pace for presentation notes, in words per minute. */
export const WORDS_PER_MINUTE = 130;

export function countWords(text) {
  return String(text || "")
    .trim()
    .split(/\s+/)
    .filter(Boolean).length;
}

/** Seconds a presenter should spend on this slide. Zero when there are no notes. */
export function readingSeconds(text) {
  const words = countWords(text);
  if (!words) return 0;
  return Math.round((words / WORDS_PER_MINUTE) * 60);
}

export function formatClock(seconds) {
  const total = Math.max(0, Math.round(seconds));
  const minutes = Math.floor(total / 60);
  const rest = total % 60;
  return `${minutes}:${String(rest).padStart(2, "0")}`;
}

/**
 * Compares elapsed time on a slide with its reading estimate.
 * Returns "on", "slow" (over budget), "fast" (well under) or "none".
 */
export function paceState(elapsedSeconds, expectedSeconds) {
  if (!expectedSeconds) return "none";
  const ratio = elapsedSeconds / expectedSeconds;
  if (ratio > 1.18) return "slow";
  if (ratio < 0.82) return "fast";
  return "on";
}
