// Plain-language names for the blocks of a slide.
//
// The element tree used to print `DIV #slyde-3`. A teacher or a founder does not
// think in tags, so the Design panel shows "Title", "Paragraph" or "List · 3
// points", and keeps the tag and handle for Developer mode only.

const KINDS = {
  h1: "title",
  h2: "title",
  h3: "heading",
  h4: "heading",
  h5: "heading",
  h6: "heading",
  p: "paragraph",
  blockquote: "quote",
  figure: "image",
  img: "image",
  svg: "graphic",
  canvas: "graphic",
  pre: "code",
  table: "table",
  ul: "list",
  ol: "list",
  button: "button",
  a: "link",
};

/** Tags that are only structure: shown when they hold text of their own. */
const CONTAINERS = new Set(["div", "section", "article", "header", "footer", "main", "span"]);

/**
 * @param {{id: string, tag: string, text?: string}[]} nodes flat, in document order
 * @returns {{id: string, kind: string, tag: string, count?: number, text: string}[]}
 */
export function describeStructure(nodes) {
  const out = [];
  for (let index = 0; index < nodes.length; index += 1) {
    const node = nodes[index];
    const tag = String(node.tag || "").toLowerCase();
    if (tag === "li" || tag === "br" || tag === "style" || tag === "script") continue;
    const text = (node.text || "").trim();
    let kind = KINDS[tag];
    if (!kind) {
      if (CONTAINERS.has(tag) && text && !hasChildBlocks(nodes, index)) kind = "text";
      else continue;
    }
    const entry = { id: node.id, kind, tag, text };
    if (kind === "list") {
      let count = 0;
      for (let next = index + 1; next < nodes.length && nodes[next].tag.toLowerCase() === "li"; next += 1) {
        count += 1;
      }
      entry.count = count;
    }
    out.push(entry);
  }
  return out;
}

/** A container with a styled child element is a wrapper, not a text block. */
function hasChildBlocks(nodes, index) {
  const next = nodes[index + 1];
  return Boolean(next && KINDS[String(next.tag).toLowerCase()] && next.text && nodes[index].text === next.text);
}

/** The words the panel shows for one entry. */
export function labelFor(entry, t) {
  if (entry.kind === "list") return t("element.list", { n: entry.count || 0 });
  return t(`element.${entry.kind}`);
}
