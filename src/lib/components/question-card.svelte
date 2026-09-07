<script lang="ts">
  import { onMount } from "svelte";
  import type { Question } from "$lib/types";
  import { Button } from "$lib/components/ui/button";
  import { Checkbox } from "$lib/components/ui/checkbox";
  import MathText from "$lib/components/math-text.svelte";
  import { formatMarks } from "$lib/utils";
  import { Flag } from "@lucide/svelte";
  import ScrollIndicator from "$lib/components/scroll-indicator.svelte";
  import {
    loadTaxonomyLabels,
    type TaxonomyLabels,
  } from "$lib/services/taxonomy-labels";

  interface Props {
    question: Question;
    index: number;
    total: number;
    answer: string | string[] | null;
    isFlagged: boolean;
    allowTextSelection?: boolean;
    showFeedback?: boolean;
    showTags?: boolean;
    readOnly?: boolean;
    onAnswer: (answer: string | string[] | null) => void;
    onToggleFlag: () => void;
  }

  let {
    question,
    index,
    total,
    answer,
    isFlagged,
    allowTextSelection = false,
    showFeedback = false,
    showTags = false,
    readOnly = false,
    onAnswer,
    onToggleFlag,
  }: Props = $props();

  let selectedOptions = $state<string[]>([]);
  let textAnswer = $state("");
  let workspaceElement = $state<HTMLElement | null>(null);
  let promptElement = $state<HTMLElement | null>(null);
  let answerElement = $state<HTMLElement | null>(null);
  let taxonomyLabels = $state<TaxonomyLabels | null>(null);

  onMount(() => {
    let mounted = true;
    void loadTaxonomyLabels().then((labels) => {
      if (mounted) taxonomyLabels = labels;
    });
    return () => {
      mounted = false;
    };
  });

  function legacyDisplayTags(): string[] {
    return (question.tags ?? [])
      .filter((tag) => !/^section\s+/i.test(tag.trim()))
      .slice(0, 4);
  }

  const displayTags = $derived.by(() => {
    const mainTag = question.taxonomy?.mainTag;
    const ids = question.taxonomy?.subtags ?? [];
    if (mainTag === undefined || taxonomyLabels === null) {
      return legacyDisplayTags();
    }
    const resolved = [
      taxonomyLabels.mainTags.get(mainTag),
      ...ids.slice(0, 4).map((id) => taxonomyLabels?.subtags.get(id)),
    ].filter((tag): tag is string => Boolean(tag));
    return resolved.length > 0 ? resolved : legacyDisplayTags();
  });

  $effect(() => {
    question.id;

    if (Array.isArray(answer)) {
      selectedOptions = [...answer];
      textAnswer = "";
    } else if (typeof answer === "string") {
      textAnswer = answer;
      selectedOptions = [answer];
    } else {
      selectedOptions = [];
      textAnswer = "";
    }
  });

  $effect(() => {
    question.id;
    promptElement?.scrollTo({ top: 0, behavior: "auto" });
    answerElement?.scrollTo({ top: 0, behavior: "auto" });
  });

  function handleSingleChoice(value: string) {
    if (selectedOptions.includes(value)) {
      selectedOptions = [];
      onAnswer(null);
      return;
    }

    selectedOptions = [value];
    onAnswer(value);
  }

  function handleMultipleChoice(optionId: string, checked: boolean) {
    let nextSelected: string[];

    if (checked) {
      nextSelected = [...new Set([...selectedOptions, optionId])];
    } else {
      nextSelected = selectedOptions.filter((id) => id !== optionId);
    }

    selectedOptions = nextSelected;
    onAnswer(nextSelected.length > 0 ? nextSelected : null);
  }

  function handleTextInput(value: string) {
    textAnswer = value;
    onAnswer(value.trim() || null);
  }

  function isCorrectAnswer(optionId: string): boolean {
    return question.correctAnswers?.includes(optionId) ?? false;
  }

  function getOptionClass(optionId: string): string {
    if (!showFeedback) return "";

    const isSelected = selectedOptions.includes(optionId);
    const isCorrect = isCorrectAnswer(optionId);

    if (isCorrect) return "border-chart-2 bg-chart-2/5";
    if (isSelected && !isCorrect) return "border-destructive bg-destructive/5";
    return "";
  }

  const legacyCodeLabels = $derived.by(() => {
    const match = question.question.match(/Codes?\s*\(([^)]+)\)\s*:/i);
    if (match) return match[1]?.trim().split(/\s+/).filter(Boolean) ?? [];

    const firstOption = question.options?.[0]?.text ?? "";
    const labelledCodes = [
      ...firstOption.matchAll(/(?:^|,)\s*([A-Z])\s*[-–]\s*/g),
    ].map((entry) => entry[1]!);
    return labelledCodes.length > 1 ? labelledCodes : [];
  });

  const legacyPairLabels = $derived.by(() => {
    if (!/\b(?:pair|pairs|matched)\b/i.test(question.question)) return [];
    const pairs = (question.options ?? []).map((option) =>
      option.text.split(/\s+:\s+/),
    );
    if (pairs.length === 0 || pairs.some((pair) => pair.length !== 2))
      return [];
    return ["List-I", "List-II"];
  });

  const displayOptions = $derived(
    (question.options ?? []).map((option) => {
      if (option.cells?.length) return option;
      const labels =
        legacyCodeLabels.length > 1 ? legacyCodeLabels : legacyPairLabels;
      if (labels.length < 2) return option;
      const values =
        legacyCodeLabels.length > 1
          ? option.text.includes(",")
            ? [
                ...option.text.matchAll(/(?:^|,)\s*[A-Z]\s*[-–]\s*([^,]+)/g),
              ].map((entry) => entry[1]!.trim())
            : option.text.trim().split(/\s+/)
          : option.text.split(/\s+:\s+/).map((value) => value.trim());
      if (values.length !== labels.length) return option;
      return {
        ...option,
        cells: values.map((text, index) => ({
          label: labels[index]!,
          text,
        })),
      };
    }),
  );

  const optionCellLabels = $derived(
    displayOptions
      ?.find((option) => option.cells?.length)
      ?.cells?.map((cell) => cell.label) ?? [],
  );
  const isCodeTable = $derived(optionCellLabels.length > 2);
  const pairColumnLengths = $derived.by(() =>
    optionCellLabels.map((label, index) =>
      Math.max(
        label.length,
        ...displayOptions.map(
          (option) => option.cells?.[index]?.text.length ?? 0,
        ),
      ),
    ),
  );
  const isCompactPairTable = $derived(
    optionCellLabels.length === 2 &&
      pairColumnLengths.every((length) => length <= 28),
  );
  const optionTableStyle = $derived(
    isCompactPairTable
      ? `--option-table-left-width: ${(pairColumnLengths[0]! * 0.62 + 0.5).toFixed(2)}rem; --option-table-right-width: ${(pairColumnLengths[1]! * 0.62 + 0.5).toFixed(2)}rem;`
      : undefined,
  );
</script>

<div
  class="question-card flex h-full min-h-0 w-full min-w-0 max-w-none flex-col"
>
  <div class="flex items-start justify-between gap-4 pb-6">
    <div class="space-y-1">
      <div
        class="flex items-center gap-1.5 text-xs font-bold uppercase tracking-[0.16em] text-muted-foreground/75"
      >
        <span>{question.type}</span>
      </div>
      <div class="flex items-center gap-3 text-xs font-bold tracking-widest">
        <span class="text-chart-2">+{formatMarks(question.marks)}</span>
        {#if question.negativeMarks > 0}
          <span class="text-destructive"
            >-{formatMarks(question.negativeMarks)}</span
          >
        {/if}
      </div>
      {#if showTags && displayTags.length}
        <div class="mt-3 flex flex-wrap gap-x-2.5 gap-y-1.5">
          {#each displayTags as tag}
            <span
              class="rounded-md border border-border/35 px-2 py-1 text-[0.62rem] font-semibold uppercase tracking-[0.1em] text-muted-foreground/80"
              >{tag}</span
            >
          {/each}
        </div>
      {/if}
    </div>
    <Button
      variant="ghost"
      size="icon"
      onclick={onToggleFlag}
      aria-label={isFlagged ? "Unflag question" : "Flag question"}
      title={isFlagged ? "Flagged" : "Flag question"}
      class="h-9 w-9 rounded-full transition-[background-color,border-color,color,box-shadow,transform] {isFlagged
        ? 'border-flag/35 bg-flag/12 text-flag shadow-sm'
        : 'text-muted-foreground/70 hover:text-foreground/85'}"
    >
      <Flag class="h-4 w-4" />
    </Button>
  </div>

  <div class="relative min-h-0 flex-1">
    <div bind:this={workspaceElement} class="question-card__workspace">
      <div class="question-card__column question-card__column--prompt">
        <section
          bind:this={promptElement}
          class="question-card__pane question-card__pane--prompt"
        >
          {#key `prompt-${question.id}`}
            <div class="question-card__content-enter">
              <div
                class={`question-card__question whitespace-pre-wrap ${allowTextSelection ? "select-text" : ""}`}
              >
                <MathText text={question.question} />
              </div>
            </div>
          {/key}
        </section>
        <ScrollIndicator
          scroller={promptElement}
          updateTrigger={question.id}
          right={2}
          insetY={0}
        />
      </div>

      <div class="question-card__column question-card__column--answer">
        <section
          bind:this={answerElement}
          class="question-card__pane question-card__pane--answer"
        >
          {#key `answer-${question.id}`}
            <div class="question-card__content-enter">
              <div class="answer-panel">
                {#if question.type === "single-choice" || question.type === "true-false"}
                  <div
                    class="space-y-2.5"
                    style={optionTableStyle}
                    role="group"
                    aria-label="Answer choices"
                  >
                    {#if optionCellLabels.length > 0}
                      <div
                        class="answer-option-table__header {isCodeTable
                          ? 'answer-option-table__header--codes'
                          : isCompactPairTable
                            ? 'answer-option-table__header--compact'
                            : ''}"
                        aria-hidden="true"
                      >
                        {#if isCodeTable}
                          <span class="answer-option-table__title">Codes:</span>
                        {/if}
                        {#each optionCellLabels as label, labelIndex}
                          <span class="answer-option-table__heading"
                            >{label}</span
                          >
                          {#if !isCodeTable && labelIndex < optionCellLabels.length - 1}
                            <span
                              class="answer-option-table__separator answer-option-table__separator--header"
                            ></span>
                          {/if}
                        {/each}
                      </div>
                    {/if}
                    {#each displayOptions as option}
                      {@const isSelected = selectedOptions.includes(option.id)}
                      <button
                        type="button"
                        class="group answer-option w-full text-left {option
                          .cells?.length
                          ? 'answer-option--table'
                          : ''} {showFeedback
                          ? getOptionClass(option.id)
                          : isSelected
                            ? 'answer-option--selected'
                            : 'answer-option--idle'}"
                        onclick={() => handleSingleChoice(option.id)}
                        disabled={readOnly}
                        aria-pressed={isSelected}
                      >
                        <div class="answer-option__leading">
                          <div
                            class="answer-option__control {isSelected
                              ? 'answer-option__control--selected'
                              : 'answer-option__control--idle'}"
                          >
                            <div
                              class="answer-option__control-dot {isSelected
                                ? 'answer-option__control-dot--visible'
                                : ''}"
                            ></div>
                          </div>
                        </div>
                        <div
                          class={`answer-option__text ${allowTextSelection ? "select-text" : ""}`}
                        >
                          {#if option.cells?.length}
                            <div
                              class="answer-option-table__cells {isCodeTable
                                ? 'answer-option-table__cells--codes'
                                : isCompactPairTable
                                  ? 'answer-option-table__cells--compact'
                                  : ''}"
                            >
                              {#each option.cells as cell, cellIndex}
                                <div class="answer-option-table__cell">
                                  <MathText text={cell.text} />
                                </div>
                                {#if !isCodeTable && cellIndex < option.cells.length - 1}
                                  <span
                                    class="answer-option-table__separator"
                                    aria-hidden="true">:</span
                                  >
                                {/if}
                              {/each}
                            </div>
                          {:else}
                            <MathText text={option.text} />
                          {/if}
                        </div>
                        {#if showFeedback && isCorrectAnswer(option.id)}
                          <div class="answer-option__feedback-dot"></div>
                        {/if}
                      </button>
                    {/each}
                  </div>
                {:else if question.type === "multiple-choice"}
                  <div class="space-y-2.5">
                    {#each question.options || [] as option}
                      <div
                        class="group answer-option {showFeedback
                          ? getOptionClass(option.id)
                          : selectedOptions.includes(option.id)
                            ? 'answer-option--selected'
                            : 'answer-option--idle'}"
                      >
                        <div class="answer-option__leading">
                          <Checkbox
                            id={`option-${question.id}-${option.id}`}
                            checked={selectedOptions.includes(option.id)}
                            onCheckedChange={(checked) =>
                              handleMultipleChoice(option.id, !!checked)}
                            disabled={readOnly}
                            class="h-4 w-4 border-2"
                          />
                        </div>
                        <label
                          for={`option-${question.id}-${option.id}`}
                          class={`answer-option__text cursor-pointer ${allowTextSelection ? "select-text" : ""}`}
                        >
                          <MathText text={option.text} />
                        </label>
                        {#if showFeedback && isCorrectAnswer(option.id)}
                          <div class="answer-option__feedback-dot"></div>
                        {/if}
                      </div>
                    {/each}
                  </div>
                {:else if question.type === "fill-blank" || question.type === "numerical"}
                  {@const isOpenEnded =
                    question.isOpenEnded === true ||
                    question.correctAnswers?.[0] === "__open__"}
                  <div class="space-y-4">
                    {#if isOpenEnded}
                      <div
                        class="answer-option answer-option--input items-start"
                      >
                        <div
                          class="answer-option__leading answer-option__leading--spacer"
                          aria-hidden="true"
                        ></div>
                        <label class="sr-only" for={`answer-${question.id}`}
                          >Practice answer notes</label
                        >
                        <textarea
                          id={`answer-${question.id}`}
                          value={textAnswer}
                          oninput={(e) =>
                            handleTextInput(e.currentTarget.value)}
                          disabled={readOnly}
                          placeholder="Draft your answer (practice notes only)"
                          rows="8"
                          class="answer-option__input min-h-[10rem] resize-y"
                        ></textarea>
                      </div>
                    {:else}
                      <div class="answer-option answer-option--input">
                        <div
                          class="answer-option__leading answer-option__leading--spacer"
                          aria-hidden="true"
                        ></div>
                        <label class="sr-only" for={`answer-${question.id}`}
                          >Answer</label
                        >
                        <input
                          id={`answer-${question.id}`}
                          type={question.type === "numerical"
                            ? "number"
                            : "text"}
                          value={textAnswer}
                          oninput={(e) =>
                            handleTextInput(e.currentTarget.value)}
                          disabled={readOnly}
                          placeholder="Answer"
                          class="answer-option__input"
                        />
                      </div>
                      {#if showFeedback}
                        <div class="answer-panel__feedback-card">
                          <span
                            class="text-[0.7rem] font-bold uppercase tracking-[0.18em] text-muted-foreground/70"
                            >Correct Answer</span
                          >
                          <span class="text-lg font-semibold text-chart-2"
                            >{question.correctAnswers?.[0] ?? ""}</span
                          >
                        </div>
                      {/if}
                    {/if}
                  </div>
                {/if}

                {#if showFeedback && question.explanation}
                  <div class="answer-panel__feedback-card border-t-0 pt-0">
                    <p
                      class="text-[0.7rem] font-bold uppercase tracking-[0.18em] text-muted-foreground/70"
                    >
                      Explanation
                    </p>
                    <p
                      class="max-w-2xl text-base leading-relaxed text-muted-foreground/82"
                    >
                      {question.explanation}
                    </p>
                  </div>
                {/if}
              </div>
            </div>
          {/key}
        </section>
      </div>
    </div>
    <ScrollIndicator scroller={workspaceElement} updateTrigger={question.id} />
  </div>
</div>

<style>
  .question-card__workspace {
    display: flex;
    min-height: 0;
    flex: 1;
    flex-direction: column;
    gap: 1.25rem;
    overflow-y: auto;
    scrollbar-width: none;
    -ms-overflow-style: none;
  }

  .question-card__workspace::-webkit-scrollbar,
  .question-card__pane::-webkit-scrollbar {
    display: none;
  }

  .question-card__pane {
    min-height: 0;
    min-width: 0;
    flex: 1 1 auto;
    scrollbar-width: none;
    -ms-overflow-style: none;
  }

  .question-card__column {
    position: relative;
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    overflow: hidden;
  }

  .question-card__pane--prompt {
    padding-bottom: 1.25rem;
    border-bottom: 1px solid color-mix(in oklab, var(--border) 82%, transparent);
  }

  .question-card__pane--answer {
    padding-top: 0.25rem;
  }

  .question-card__question {
    max-width: min(66ch, 100%);
    font-size: clamp(0.98rem, 0.94rem + 0.18vw, 1.12rem);
    font-weight: 500;
    line-height: 1.55;
    letter-spacing: -0.02em;
    text-wrap: pretty;
    color: color-mix(in oklab, var(--foreground) 92%, transparent);
  }

  .question-card__question :global(.math-text__table) {
    width: min(100%, var(--math-table-width, 100%));
    min-width: min(100%, 28rem);
    table-layout: fixed;
  }

  .question-card__question :global(.math-text__table th),
  .question-card__question :global(.math-text__table td) {
    white-space: normal;
    vertical-align: top;
  }

  .question-card__question :global(.math-text__table--2-column th:first-child),
  .question-card__question :global(.math-text__table--2-column td:first-child) {
    width: 34%;
  }

  .question-card__question :global(.math-text__table--adaptive th:first-child),
  .question-card__question :global(.math-text__table--adaptive td:first-child) {
    width: var(--math-table-first-column, 34%);
  }

  .question-card__question :global(.math-text__table--compact th),
  .question-card__question :global(.math-text__table--compact td) {
    padding: 0.52em 0.78em;
  }

  .question-card__question :global(.math-text__table--regular th),
  .question-card__question :global(.math-text__table--regular td) {
    padding: 0.48em 0.72em;
  }

  .question-card__question :global(.math-text__table--serial th:first-child),
  .question-card__question :global(.math-text__table--serial td:first-child) {
    width: 3.5rem;
  }

  .question-card__content-enter {
    animation: question-card-content-swap 160ms cubic-bezier(0.22, 1, 0.36, 1);
    will-change: opacity, transform;
  }

  .answer-panel {
    display: flex;
    min-height: 0;
    flex: 1;
    flex-direction: column;
    gap: 0.9rem;
  }

  .answer-option {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    align-items: start;
    gap: 0.9rem;
    border: 1px solid color-mix(in oklab, var(--border) 74%, transparent);
    background: color-mix(in oklab, var(--background) 97%, var(--card));
    padding: 0.95rem 1rem;
    transition:
      border-color 0.18s ease,
      background 0.18s ease;
  }

  .answer-option:hover {
    border-color: color-mix(in oklab, var(--foreground) 18%, var(--border));
    background: color-mix(in oklab, var(--background) 93%, var(--muted));
  }

  .answer-option--idle {
    background: color-mix(in oklab, var(--background) 97%, var(--card));
  }

  .answer-option--selected {
    border-color: color-mix(in oklab, var(--foreground) 22%, var(--border));
    background: color-mix(in oklab, var(--background) 88%, var(--muted));
    box-shadow:
      0 0 0 1px rgba(255, 255, 255, 0.08) inset,
      0 0 18px rgba(255, 255, 255, 0.08);
  }

  .answer-option__leading {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    /* Sit with the first line of option text (grid is items-start) */
    padding-top: 0.28rem;
  }

  .answer-option__leading--spacer {
    width: 1rem;
    flex-shrink: 0;
  }

  .answer-option__control {
    display: flex;
    height: 1rem;
    width: 1rem;
    flex-shrink: 0;
    align-items: center;
    justify-content: center;
    border-radius: 999px;
    border: 2px solid currentColor;
    transition:
      color 0.18s ease,
      border-color 0.18s ease;
  }

  .answer-option__control--idle {
    color: color-mix(in oklab, var(--muted-foreground) 78%, transparent);
  }

  .answer-option__control--selected {
    color: color-mix(in oklab, var(--foreground) 92%, transparent);
  }

  .answer-option__control-dot {
    height: 0.45rem;
    width: 0.45rem;
    border-radius: 999px;
    background: currentColor;
    opacity: 0;
    transform: scale(0.55);
    transition:
      opacity 0.18s ease,
      transform 0.18s ease;
  }

  .answer-option__control-dot--visible {
    opacity: 1;
    transform: scale(1);
  }

  .answer-option__text {
    min-width: 0;
    padding: 0.1rem 0;
    font-size: clamp(0.95rem, 0.92rem + 0.12vw, 1.05rem);
    font-weight: 500;
    line-height: 1.42;
    letter-spacing: -0.015em;
    color: color-mix(in oklab, var(--foreground) 95%, transparent);
  }

  .answer-option-table__header {
    position: relative;
    display: grid;
    grid-template-columns: minmax(0, 4fr) auto minmax(7rem, 1fr);
    align-items: baseline;
    gap: 0;
    padding: 0 1.9rem 0.15rem 2.9rem;
    color: color-mix(in srgb, var(--muted-foreground) 76%, transparent);
    font-size: 0.78rem;
    font-weight: 600;
    letter-spacing: 0.055em;
    text-transform: uppercase;
  }

  .answer-option-table__title {
    position: absolute;
    left: 1rem;
    top: 0;
    color: color-mix(in srgb, var(--muted-foreground) 76%, transparent);
    font-size: 0.78rem;
    font-weight: 600;
    letter-spacing: 0.055em;
    text-transform: uppercase;
  }

  .answer-option-table__header.answer-option-table__header--codes,
  .answer-option-table__cells.answer-option-table__cells--codes {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 0;
  }

  .answer-option-table__header--codes {
    padding-right: 1.9rem;
    padding-left: 2.9rem;
  }

  .answer-option-table__header--codes .answer-option-table__heading,
  .answer-option-table__cells--codes .answer-option-table__cell {
    text-align: center;
  }

  .answer-option-table__heading,
  .answer-option-table__cell {
    min-width: 0;
    justify-self: stretch;
    text-align: left;
  }

  .answer-option-table__cell :global(.math-text),
  .answer-option-table__cell :global(.math-text__line) {
    text-align: left;
  }

  .answer-option-table__cells--codes
    .answer-option-table__cell
    :global(.math-text),
  .answer-option-table__cells--codes
    .answer-option-table__cell
    :global(.math-text__line) {
    text-align: center;
  }

  .answer-option-table__cells {
    display: grid;
    grid-template-columns: minmax(0, 4fr) auto minmax(7rem, 1fr);
    align-items: baseline;
    gap: 0;
  }

  .answer-option-table__header--compact,
  .answer-option-table__cells--compact {
    grid-template-columns:
      minmax(0, var(--option-table-left-width)) auto
      minmax(0, var(--option-table-right-width));
    justify-content: start;
  }

  .answer-option-table__separator {
    margin-left: 0.35rem;
    margin-right: 0.8rem;
    color: color-mix(in oklab, var(--muted-foreground) 72%, transparent);
    letter-spacing: normal;
    text-align: center;
  }

  .answer-option-table__separator--header {
    color: transparent;
  }

  .answer-option-table__separator--header::before {
    content: ":";
  }

  .answer-option--table {
    align-items: start;
    border-radius: 0;
    padding-block: 0.78rem;
  }

  .answer-option--table .answer-option__leading {
    padding-top: 0.2rem;
  }

  .answer-option--table .answer-option__text {
    width: 100%;
    padding-block: 0;
  }

  .answer-option__feedback-dot {
    height: 0.55rem;
    width: 0.55rem;
    margin-top: 0.4rem;
    border-radius: 999px;
    background: var(--chart-2);
  }

  .answer-option--input {
    align-items: stretch;
  }

  .answer-option__input {
    width: 100%;
    height: 100%;
    min-height: calc(1.42em + 0.2rem);
    border: 0;
    outline: none;
    background: transparent;
    border: 0;
    padding: 0.1rem 0;
    font-size: clamp(1.02rem, 0.95rem + 0.22vw, 1.2rem);
    font-weight: 500;
    line-height: 1.42;
    letter-spacing: -0.02em;
    color: color-mix(in oklab, var(--foreground) 95%, transparent);
    box-shadow: none;
    appearance: none;
    -webkit-appearance: none;
    -moz-appearance: textfield;
  }

  .answer-option__input::placeholder {
    color: color-mix(in oklab, var(--muted-foreground) 74%, transparent);
  }

  .answer-option__input:focus-visible {
    outline: none;
    box-shadow: none;
  }

  .answer-option__input::-webkit-outer-spin-button,
  .answer-option__input::-webkit-inner-spin-button {
    margin: 0;
    -webkit-appearance: none;
  }

  .answer-panel__feedback-card {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    border-top: 1px solid color-mix(in oklab, var(--border) 74%, transparent);
    padding-top: 1rem;
  }

  @keyframes question-card-content-swap {
    from {
      opacity: 0;
      transform: translate3d(0, 6px, 0);
    }

    to {
      opacity: 1;
      transform: translate3d(0, 0, 0);
    }
  }

  @media (min-width: 1100px) {
    .question-card__workspace {
      position: absolute;
      inset: 0;
      display: grid;
      grid-template-columns: minmax(0, 60%) minmax(22rem, 40%);
      gap: 2rem;
      overflow: hidden;
    }

    .question-card__pane {
      overflow-y: auto;
      overscroll-behavior: contain;
      scrollbar-gutter: stable;
    }

    .question-card__column--prompt {
      border-right: 1px solid
        color-mix(in oklab, var(--foreground) 22%, var(--border));
    }

    .question-card__pane--prompt {
      padding-right: 2rem;
      padding-bottom: 0;
      border-right: 0;
      border-bottom: 0;
    }

    .question-card__pane--answer {
      min-height: 0;
      padding-left: 0.25rem;
      padding-right: 0;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .question-card__content-enter {
      animation: none;
    }
  }
</style>
