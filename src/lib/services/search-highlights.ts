import type { SearchTerm } from "$lib/types";

/** Mark visible words only. Queries never become HTML or regular expressions. */
export function highlightSearchTerms(root: HTMLElement, terms: SearchTerm[]) {
  for (const mark of root.querySelectorAll("mark[data-search-match]")) {
    mark.replaceWith(document.createTextNode(mark.textContent ?? ""));
  }
  root.normalize();
  if (terms.length === 0) return;

  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
    acceptNode(node) {
      // KaTeX has duplicate visual/accessibility trees. Keep both intact.
      return node.parentElement?.closest(".katex, math, script, style, mark")
        ? NodeFilter.FILTER_REJECT
        : NodeFilter.FILTER_ACCEPT;
    },
  });
  const nodes: Text[] = [];
  while (walker.nextNode()) nodes.push(walker.currentNode as Text);
  for (const node of nodes) {
    const text = node.data;
    const matches = Array.from(text.matchAll(/[\p{L}\p{N}\p{M}]+/gu)).filter(
      ([word]) => {
        const normalized = word.toLowerCase();
        return terms.some(
          (term) =>
            normalized === term.text ||
            (term.prefix && normalized.startsWith(term.text)),
        );
      },
    );
    if (matches.length === 0) continue;
    const fragment = document.createDocumentFragment();
    let offset = 0;
    for (const match of matches) {
      fragment.append(document.createTextNode(text.slice(offset, match.index)));
      const mark = document.createElement("mark");
      mark.dataset.searchMatch = "";
      mark.textContent = match[0];
      fragment.append(mark);
      offset = match.index + match[0].length;
    }
    fragment.append(document.createTextNode(text.slice(offset)));
    node.replaceWith(fragment);
  }
}
