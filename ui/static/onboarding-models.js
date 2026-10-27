// Picks a sensible default model so a first-time user never meets a model list.
//
// Providers publish dozens of ids (embeddings, audio, previews, tiny and huge
// variants). The wizard needs one that writes slides well and answers quickly;
// the full list stays one click away in the model sheet.

/** Ordered patterns per provider: the first pattern with a match wins. */
const PREFERRED = {
  deepseek: [/^deepseek-chat$/, /^deepseek-v\d/, /^deepseek-reasoner$/],
  openai: [/^gpt-5(?:\.\d+)?$/, /^gpt-4\.1$/, /^gpt-4o$/, /^gpt-4o-mini$/],
  anthropic: [/claude-.*sonnet/, /claude-.*opus/, /claude-.*haiku/],
  gemini: [/gemini-[\d.]+-flash$/, /gemini-[\d.]+-pro$/, /flash/],
  xai: [/^grok-\d+$/, /^grok-/],
  groq: [/llama-3\.3-70b/, /llama-3\.1-70b/, /llama/],
  openrouter: [/:free$/, /deepseek/, /claude.*sonnet/],
  nvidia: [/llama-3\.3-70b/, /llama-3\.1-70b/, /llama/],
};

/** Ids that are never a good chat model for writing slides. */
const UNSUITABLE = /embed|whisper|tts|audio|realtime|moderation|image|vision-preview|guard|rerank|transcribe|dall-e/i;

/** Version numbers of an id ("claude-sonnet-4-5" -> [4, 5]); release dates are ignored. */
function versionOf(id) {
  return (id.match(/\d+/g) || []).filter((part) => part.length !== 8).map(Number);
}

function compareVersions(a, b) {
  const left = versionOf(a);
  const right = versionOf(b);
  for (let index = 0; index < Math.max(left.length, right.length); index += 1) {
    const difference = (right[index] ?? -1) - (left[index] ?? -1);
    if (difference) return difference;
  }
  return b.localeCompare(a);
}

const newest = (ids) => [...ids].sort(compareVersions)[0];

/**
 * @param {string} providerId
 * @param {{id: string, free?: boolean}[]} models
 * @returns {string} a model id, or "" when the list is empty
 */
export function recommendedModel(providerId, models) {
  const usable = (models || []).map((model) => model.id).filter((id) => id && !UNSUITABLE.test(id));
  if (!usable.length) return "";
  for (const pattern of PREFERRED[providerId] || []) {
    const matches = usable.filter((id) => pattern.test(id));
    if (matches.length) return newest(matches);
  }
  return usable[0];
}

/** The providers the wizard shows up front; the rest live in the model sheet. */
export const FEATURED_PROVIDERS = ["deepseek", "openai", "anthropic", "gemini"];
