<script lang="ts">
  import { tick } from "svelte";
  import MathText from "$lib/components/math-text.svelte";
  import { highlightSearchTerms } from "$lib/services/search-highlights";
  import type { SearchTerm } from "$lib/types";

  let {
    text,
    terms,
    class: className = "",
  }: {
    text: string;
    terms: SearchTerm[];
    class?: string;
  } = $props();
  let element: HTMLDivElement;

  $effect(() => {
    // Wait for MathText's HTML update, and discard work for superseded text.
    const current = { text, terms };
    let cancelled = false;
    void tick().then(() => {
      if (!cancelled) highlightSearchTerms(element, current.terms);
    });
    return () => {
      cancelled = true;
    };
  });
</script>

<div bind:this={element} class={className}>
  <MathText {text} />
</div>

<style>
  :global(mark[data-search-match]) {
    background: var(--accent);
    color: var(--foreground);
    font-weight: 650;
    text-decoration: underline;
    text-decoration-color: var(--muted-foreground);
    text-underline-offset: 3px;
  }
</style>
