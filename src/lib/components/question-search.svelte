<script lang="ts">
  import { onDestroy, onMount, tick } from "svelte";
  import SearchText from "$lib/components/search-text.svelte";
  import ScrollIndicator from "$lib/components/scroll-indicator.svelte";
  import { Button } from "$lib/components/ui/button";
  import {
    Dialog,
    DialogContent,
    DialogTitle,
  } from "$lib/components/ui/dialog";
  import {
    cancelQuestionSearch,
    searchQuestions,
  } from "$lib/services/question-search";
  import type {
    QuestionSearchResponse,
    QuestionSearchResult,
  } from "$lib/types";
  import { LoaderCircle, Search } from "@lucide/svelte";

  interface Props {
    open?: boolean;
    enabled?: boolean;
    sections?: string[];
    scopeLabel?: string;
  }

  type PendingSearch = {
    value: string;
    generation: number;
    sections: string[];
    scopeKey: string;
  };

  let {
    open = $bindable(false),
    enabled = true,
    sections = [],
    scopeLabel = "All Papers",
  }: Props = $props();

  let query = $state("");
  let originalSpellingRequest = $state<{
    inputValue: string;
    searchValue: string;
  } | null>(null);
  let response = $state<QuestionSearchResponse | null>(null);
  let isSearching = $state(false);
  let isComposing = $state(false);
  let error = $state<string | null>(null);
  let inputElement = $state<HTMLInputElement | null>(null);
  let resultsScrollElement = $state<HTMLElement | null>(null);
  let searchTimer: ReturnType<typeof setTimeout> | null = null;
  let requestGeneration = 0;
  let searchClientId = "";
  let activeScopeKey = "";
  let wasOpen = false;

  const trimmedQuery = $derived(query.trim());
  // The backend quotes terms to suppress spelling recovery. Keep that syntax
  // out of the editable text, and apply it only to the unchanged original input.
  const searchQuery = $derived(
    originalSpellingRequest?.inputValue === query
      ? originalSpellingRequest.searchValue
      : trimmedQuery,
  );
  const sectionKey = $derived(sections.join("\u001f"));
  const showResultsPanel = $derived(
    trimmedQuery.length >= 2 && (response !== null || error !== null),
  );
  const currentResponse = $derived(response?.query.trim() === searchQuery);
  const highlightTerms = $derived(response?.highlightTerms ?? []);

  function useOriginalSpelling() {
    if (!currentResponse || !response?.originalSpellingQuery) return;
    originalSpellingRequest = {
      inputValue: query,
      searchValue: response.originalSpellingQuery,
    };
    inputElement?.focus();
  }

  function correctionLabel(corrected: string): string {
    const originalWords = query.match(/[\p{L}\p{N}]+/gu) ?? [];
    let wordIndex = 0;
    return corrected.replace(/[\p{L}\p{N}]+/gu, (word) => {
      const original = originalWords[wordIndex++];
      if (!original) return word;
      if (original === original.toLowerCase()) return word.toLowerCase();
      if (original === original.toUpperCase()) return word.toUpperCase();

      // Preserve title case and mixed capitalization without changing the
      // actual query or the normalized spelling used by the search service.
      const originalLetters = Array.from(original);
      return Array.from(word, (letter, index) => {
        const source = originalLetters[index];
        return source && source !== source.toLowerCase()
          ? letter.toUpperCase()
          : letter.toLowerCase();
      }).join("");
    });
  }

  function handleGlobalKeydown(event: KeyboardEvent) {
    if (event.isComposing || isComposing) return;
    if (
      enabled &&
      (event.metaKey || event.ctrlKey) &&
      event.key.toLowerCase() === "k"
    ) {
      event.preventDefault();
      open = true;
      return;
    }

    if (
      !open ||
      event.target === inputElement ||
      (event.key !== "Backspace" && event.key !== "Delete") ||
      event.metaKey ||
      event.ctrlKey ||
      event.altKey
    ) {
      return;
    }

    // Clicking or scrolling results can move focus away from the field. Keep
    // deletion behaving like an active search instead of invoking page-back.
    event.preventDefault();
    const input = inputElement;
    const selectionStart = input?.selectionStart ?? query.length;
    const selectionEnd = input?.selectionEnd ?? selectionStart;
    let deleteStart = selectionStart;
    let deleteEnd = selectionEnd;

    if (selectionStart === selectionEnd && event.key === "Backspace") {
      const previousCharacter = Array.from(query.slice(0, selectionStart)).at(
        -1,
      );
      deleteStart = Math.max(
        0,
        selectionStart - (previousCharacter?.length ?? 0),
      );
    } else if (selectionStart === selectionEnd && event.key === "Delete") {
      const nextCharacter = Array.from(query.slice(selectionEnd))[0];
      deleteEnd = Math.min(
        query.length,
        selectionEnd + (nextCharacter?.length ?? 0),
      );
    }

    originalSpellingRequest = null;
    query = query.slice(0, deleteStart) + query.slice(deleteEnd);
    void tick().then(() => {
      input?.focus();
      input?.setSelectionRange(deleteStart, deleteStart);
    });
  }

  function resetPendingSearch() {
    requestGeneration += 1;
    if (searchClientId) {
      void cancelQuestionSearch(searchClientId, requestGeneration).catch(() => {
        // Generation checks still reject stale replies if cancellation fails.
      });
    }
    if (searchTimer) {
      clearTimeout(searchTimer);
      searchTimer = null;
    }
  }

  function queueSearch(request: PendingSearch) {
    if (!searchClientId) searchClientId = crypto.randomUUID();
    void runSearch(request);
  }

  async function runSearch(current: PendingSearch) {
    let settled = false;
    let accepted = false;
    const isCurrent = () =>
      current.generation === requestGeneration &&
      searchQuery === current.value &&
      sectionKey === current.scopeKey;
    async function accept(next: QuestionSearchResponse) {
      if (!isCurrent() || next.query.trim() !== current.value) return;
      const firstReply = !accepted;
      const scroller = resultsScrollElement;
      const viewportTop = scroller?.getBoundingClientRect().top ?? 0;
      const anchor =
        !firstReply && scroller && scroller.scrollTop > 0
          ? Array.from(
              scroller.querySelectorAll<HTMLElement>("[data-question-id]"),
            ).find((item) => item.getBoundingClientRect().bottom > viewportTop)
          : undefined;
      const anchorOffset = anchor
        ? anchor.getBoundingClientRect().top - viewportTop
        : 0;
      accepted = true;
      response = next;
      error = null;
      await tick();
      // Semantic completion must not pull a reader back to the top.
      if (firstReply && isCurrent() && resultsScrollElement) {
        resultsScrollElement.scrollTop = 0;
      } else if (anchor && isCurrent() && resultsScrollElement) {
        const retained = Array.from(
          resultsScrollElement.querySelectorAll<HTMLElement>(
            "[data-question-id]",
          ),
        ).find((item) => item.dataset.questionId === anchor.dataset.questionId);
        if (retained) {
          resultsScrollElement.scrollTop +=
            retained.getBoundingClientRect().top -
            resultsScrollElement.getBoundingClientRect().top -
            anchorOffset;
        }
      }
    }
    try {
      const next = await searchQuestions(current.value, current.sections, {
        clientId: searchClientId,
        requestId: current.generation,
        onProgress: (preview) => {
          if (!settled) void accept(preview);
        },
      });
      settled = true;
      await accept(next);
    } catch (caught) {
      settled = true;
      if (!isCurrent()) return;
      if (accepted && response?.query.trim() === current.value) {
        // Preserve usable keyword results if the second phase cannot complete.
        response = { ...response, semanticStatus: "unavailable" };
      } else {
        response = null;
        error =
          caught instanceof Error ? caught.message : "Search is unavailable";
      }
    } finally {
      if (isCurrent()) isSearching = false;
    }
  }

  function chooseSpelling(value: string) {
    originalSpellingRequest = null;
    query = correctionLabel(value);
    inputElement?.focus();
  }

  function optionsFitSingleRow(result: QuestionSearchResult): boolean {
    if (result.options.length > 4) return false;
    const optionLengths = result.options.map((option) => option.text.length);
    return (
      optionLengths.every((length) => length <= 28) &&
      optionLengths.reduce((total, length) => total + length, 0) <= 88
    );
  }

  function resultContext(result: QuestionSearchResult): string {
    return [result.stage, result.paper].filter(Boolean).join(" · ");
  }

  $effect(() => {
    const dialogOpen = open;

    if (dialogOpen && !wasOpen) {
      void tick().then(() => inputElement?.focus());
    } else if (!dialogOpen && wasOpen) {
      originalSpellingRequest = null;
      query = "";
      isComposing = false;
      response = null;
      error = null;
      isSearching = false;
      resetPendingSearch();
    }

    wasOpen = dialogOpen;
  });

  $effect(() => {
    const value = searchQuery;
    const dialogOpen = open;
    const scopeKey = sectionKey;
    const scopedSections = [...sections];
    const scopeChanged = activeScopeKey !== scopeKey;
    activeScopeKey = scopeKey;
    resetPendingSearch();

    if (scopeChanged) {
      response = null;
    }

    if (!dialogOpen || isComposing || value.length < 2) {
      isSearching = false;
      error = null;
      if (value.length < 2) {
        response = null;
      }
      return;
    }

    const generation = requestGeneration;
    isSearching = true;
    error = null;
    const request = {
      value,
      generation,
      sections: scopedSections,
      scopeKey,
    };
    if (originalSpellingRequest?.inputValue === query) {
      // A deliberate retry is ready to run; debounce only ongoing typing.
      queueSearch(request);
    } else {
      searchTimer = setTimeout(() => {
        searchTimer = null;
        queueSearch(request);
      }, 140);
    }

    return () => {
      if (searchTimer) {
        clearTimeout(searchTimer);
        searchTimer = null;
      }
    };
  });

  onMount(() => {
    window.addEventListener("keydown", handleGlobalKeydown, { capture: true });
  });

  onDestroy(() => {
    resetPendingSearch();
    window.removeEventListener("keydown", handleGlobalKeydown, {
      capture: true,
    });
  });
</script>

<Button
  variant="ghost"
  size="icon"
  class="app-chrome-enter h-9 w-9 rounded-full border border-border/55 text-muted-foreground transition-colors hover:border-foreground/35 hover:text-foreground"
  onclick={() => (open = true)}
  title={`Search ${scopeLabel}`}
  aria-label={`Search ${scopeLabel}`}
  disabled={!enabled}
>
  <Search class="h-4 w-4" />
</Button>

<Dialog bind:open>
  <DialogContent
    closeOnInteractOutside={true}
    preventScroll={false}
    showCloseButton={false}
    class="top-[clamp(5.5rem,13vh,8.5rem)] flex w-[calc(100%-2rem)] max-w-5xl translate-y-0 flex-col gap-2 border-0 bg-transparent p-0 shadow-none [--ui-label-size:0.8125rem]"
  >
    <DialogTitle class="sr-only">Search {scopeLabel}</DialogTitle>

    <div
      class="flex h-14 w-full items-center gap-3 border border-border bg-popover px-4 shadow-[0_16px_48px_rgba(0,0,0,0.14)] transition-[border-color,box-shadow] duration-150 focus-within:border-foreground/28 focus-within:shadow-[0_20px_60px_rgba(0,0,0,0.2)] dark:shadow-[0_20px_60px_rgba(0,0,0,0.42)] dark:focus-within:shadow-[0_24px_72px_rgba(0,0,0,0.52)]"
      aria-busy={isSearching}
    >
      {#if isSearching}
        <LoaderCircle
          class="h-4 w-4 shrink-0 animate-spin text-muted-foreground"
        />
      {:else}
        <Search class="h-4 w-4 shrink-0 text-muted-foreground" />
      {/if}
      <label class="sr-only" for="question-search-input">Search questions</label
      >
      <input
        id="question-search-input"
        bind:this={inputElement}
        bind:value={query}
        oninput={() => (originalSpellingRequest = null)}
        oncompositionstart={() => (isComposing = true)}
        oncompositionend={() => (isComposing = false)}
        class="h-full min-w-0 flex-1 bg-transparent text-lg font-medium tracking-[-0.012em] text-foreground outline-none placeholder:font-normal placeholder:text-muted-foreground/48"
        placeholder="Search"
        autocomplete="off"
        spellcheck="false"
      />
    </div>

    {#if showResultsPanel}
      <section
        id="question-search-results"
        aria-busy={isSearching}
        class="relative flex max-h-[65dvh] min-h-0 w-full animate-in flex-col overflow-hidden border border-border bg-popover shadow-[0_24px_70px_rgba(0,0,0,0.16)] fade-in-0 slide-in-from-top-1 duration-150 dark:shadow-[0_28px_80px_rgba(0,0,0,0.45)]"
      >
        {#if response && !error}
          <p role="status" class="sr-only">
            {#if !currentResponse || response.semanticStatus === "pending"}
              Updating results…
            {:else}
              {`${response.totalMatches.toLocaleString()} ${response.totalMatches === 1 ? "match" : "matches"} in ${scopeLabel}`}
            {/if}
          </p>
          <!-- Keep the correction with the displayed results until the retry
               replaces both. Editing the visible query still hides it. -->
          {#if response.query.trim() === trimmedQuery && response.correctedQuery}
            <div
              class="flex shrink-0 flex-wrap items-baseline gap-x-3 gap-y-0 px-4 pt-2 sm:px-5"
            >
              <p
                class="ui-small-label min-w-0 break-words text-muted-foreground/75"
              >
                Results for <span
                  class="text-base font-medium normal-case tracking-normal text-foreground/85"
                  >“{correctionLabel(response.correctedQuery)}”</span
                >
              </p>
              {#if response.originalSpellingQuery}
                <button
                  type="button"
                  class="ui-button-text group min-h-7 min-w-0 max-w-full cursor-pointer break-words py-1 text-left text-muted-foreground/75 transition-colors enabled:hover:text-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring disabled:cursor-wait"
                  aria-label={`Search for “${trimmedQuery}” without spelling correction`}
                  disabled={!currentResponse}
                  onclick={useOriginalSpelling}
                >
                  <span class="mr-3 text-muted-foreground/35" aria-hidden="true"
                    >·</span
                  >Search
                  <span
                    class="text-base font-medium normal-case tracking-normal underline decoration-foreground/25 underline-offset-4 group-hover:decoration-foreground/60"
                    >“{trimmedQuery}”</span
                  >
                </button>
              {/if}
            </div>
          {/if}
          {#if currentResponse && response.spellingAlternatives.length > 0}
            <div
              class="flex shrink-0 flex-wrap items-baseline gap-x-3 gap-y-1 px-4 pt-2 sm:px-5"
            >
              <span class="ui-small-label text-muted-foreground/75"
                >Did you mean</span
              >
              {#each response.spellingAlternatives as alternative}
                <button
                  type="button"
                  class="min-h-7 cursor-pointer break-words text-base font-medium text-foreground/85 underline decoration-foreground/25 underline-offset-4 hover:decoration-foreground/60 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
                  onclick={() => chooseSpelling(alternative)}
                  >“{correctionLabel(alternative)}”</button
                >
              {/each}
            </div>
          {/if}
          {#if currentResponse && response.semanticStatus === "unavailable"}
            <p
              class="ui-small-label px-4 pt-3 font-medium leading-relaxed text-muted-foreground/75 sm:px-5"
            >
              Related search unavailable. Showing keyword matches.
            </p>
          {/if}
        {/if}
        {#if error}
          <p role="alert" class="px-5 py-4 text-base text-destructive">
            {error}
          </p>
        {:else if response?.results.length === 0}
          <div class="space-y-2 px-4 py-5 sm:px-5">
            <h2 class="ui-small-label text-foreground/80">
              {response.semanticStatus === "pending"
                ? "Searching…"
                : "No results"}
            </h2>
            <p
              class="ui-small-label font-medium leading-relaxed text-muted-foreground/75"
            >
              {response.semanticStatus === "pending"
                ? "Looking for related questions."
                : response.spellingAlternatives.length > 0
                  ? "Choose a spelling above or edit your search"
                  : "Try fewer words or a broader topic"}
            </p>
          </div>
        {:else if response}
          <div
            bind:this={resultsScrollElement}
            class="min-h-0 overflow-y-auto no-scrollbar"
          >
            <div class="space-y-1.5 p-3 sm:p-4">
              {#each response.results as result, index (result.questionId)}
                {#if index === 0 || response.results[index - 1]?.matchStrength !== result.matchStrength}
                  <div
                    class="ui-small-label px-1 pb-1 pt-1 text-muted-foreground/60"
                  >
                    {result.matchStrength === "strong"
                      ? "Strong matches"
                      : "Related results"}
                  </div>
                {:else}
                  <div
                    class="mx-0 h-px bg-border/45 sm:mx-2"
                    aria-hidden="true"
                  ></div>
                {/if}
                <article
                  data-question-id={result.questionId}
                  class="grid gap-3 px-4 py-4 [content-visibility:auto] [contain-intrinsic-size:auto_10rem] sm:grid-cols-[7.25rem_minmax(0,1fr)] sm:gap-6 sm:px-5"
                >
                  <div class="flex flex-wrap items-baseline gap-2 sm:block">
                    {#if result.year}
                      <p
                        class="text-base font-semibold tabular-nums tracking-[-0.01em] text-foreground/78"
                      >
                        {result.year}
                      </p>
                    {/if}
                    <p
                      class="text-xs font-bold uppercase tracking-[0.13em] text-muted-foreground/50 sm:mt-1.5"
                    >
                      {resultContext(result)}
                    </p>
                    {#if result.questionNumber != null}
                      <p
                        class="text-xs font-bold uppercase tracking-[0.13em] text-muted-foreground/38 sm:mt-1"
                      >
                        Q {result.questionNumber}
                      </p>
                    {/if}
                  </div>

                  <div class="min-w-0">
                    <SearchText
                      text={result.question}
                      terms={highlightTerms}
                      class="text-lg font-medium leading-[1.55] tracking-[-0.008em] text-foreground/88"
                    />

                    {#if result.options.length > 0}
                      <ol
                        class={`mt-3 grid grid-cols-1 gap-x-6 gap-y-1 text-base leading-relaxed text-foreground/66 sm:grid-cols-2 ${optionsFitSingleRow(result) ? "lg:grid-cols-4" : ""}`}
                      >
                        {#each result.options as option}
                          <li
                            class="grid min-w-0 grid-cols-[auto_minmax(0,1fr)] items-baseline gap-1.5"
                          >
                            <span
                              class="font-semibold uppercase text-muted-foreground/48"
                            >
                              ({option.id})
                            </span>
                            <SearchText
                              text={option.text}
                              terms={highlightTerms}
                            />
                          </li>
                        {/each}
                      </ol>
                    {/if}
                    {#if result.semanticMatch && !result.lexicalMatch}
                      <p
                        class="mt-2 text-[length:var(--ui-label-size)] text-muted-foreground"
                      >
                        Related by meaning{result.mainTag
                          ? ` · ${result.mainTag}`
                          : ""}
                      </p>
                    {/if}
                  </div>
                </article>
              {/each}
            </div>
          </div>
          <ScrollIndicator
            scroller={resultsScrollElement}
            right={2}
            updateTrigger={response}
          />
        {/if}
      </section>
    {/if}
  </DialogContent>
</Dialog>
