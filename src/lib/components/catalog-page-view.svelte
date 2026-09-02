<script lang="ts">
  import { ChevronLeft, History, Home } from "@lucide/svelte";
  import CatalogScreenContent from "$lib/components/catalog-screen-content.svelte";
  import LoadingProgress from "$lib/components/loading-progress.svelte";
  import { updaterHome } from "$lib/stores/updater-home";
  import OptionalPreferencesModal from "$lib/components/optional-preferences-modal.svelte";
  import QuestionSearch from "$lib/components/question-search.svelte";
  import ScrollIndicator from "$lib/components/scroll-indicator.svelte";
  import ShortcutsLauncher from "$lib/components/shortcuts-launcher.svelte";
  import ThemeSwitcher from "$lib/components/theme-switcher.svelte";
  import type {
    CatalogScreen,
    PaperListItem,
  } from "$lib/services/catalog-model";
  import type { MainsPaperType } from "$lib/constants/upsc-catalog";
  import type { StoredQuestionBank, TestAttemptHistoryEntry } from "$lib/types";

  $effect(() => {
    updaterHome.set(screen.kind === "home" && !isLoading);
    return () => updaterHome.set(false);
  });

  interface Props {
    isLoading: boolean;
    isLoadingComplete: boolean;
    screen: CatalogScreen;
    banks: StoredQuestionBank[];
    pageTitle: string;
    pageTrail: string | null;
    screenAnimationKey: string;
    totalQuestions: number;
    prelimsCount: number;
    mainsCount: number;
    mainsPaperTypes: MainsPaperType[];
    prelimsPapers: PaperListItem[];
    mainsPapers: PaperListItem[];
    dualPaper1: PaperListItem[];
    dualPaper2: PaperListItem[];
    isDualPaper: boolean;
    historyEntries: TestAttemptHistoryEntry[];
    historyLoading: boolean;
    historyLoadingComplete: boolean;
    historyError: string | null;
    searchOpen: boolean;
    searchSections: string[];
    searchScopeLabel: string;
    searchEnabled: boolean;
    onCatalogLoadingComplete: () => void;
    onHistoryLoadingComplete: () => void;
    onBack: () => void;
    onHome: () => void;
    onOpenHistory: () => void;
    onScreenChange: (screen: CatalogScreen) => void;
    onOpenResult: (id: string) => void;
    onOpenPrelim: (bank: StoredQuestionBank) => void;
    onOpenTheory: (item: PaperListItem) => void;
  }

  let {
    isLoading,
    isLoadingComplete,
    screen,
    banks,
    pageTitle,
    pageTrail,
    screenAnimationKey,
    totalQuestions,
    prelimsCount,
    mainsCount,
    mainsPaperTypes,
    prelimsPapers,
    mainsPapers,
    dualPaper1,
    dualPaper2,
    isDualPaper,
    historyEntries,
    historyLoading,
    historyLoadingComplete,
    historyError,
    searchOpen = $bindable(),
    searchSections,
    searchScopeLabel,
    searchEnabled,
    onCatalogLoadingComplete,
    onHistoryLoadingComplete,
    onBack,
    onHome,
    onOpenHistory,
    onScreenChange,
    onOpenResult,
    onOpenPrelim,
    onOpenTheory,
  }: Props = $props();

  let scrollElement = $state<HTMLElement | null>(null);
  const historyVisible = $derived(
    screen.kind === "prelims" ||
      screen.kind === "prelims-paper" ||
      screen.kind === "prelims-history",
  );
</script>

{#if isLoading}
  <LoadingProgress
    class="h-full bg-background"
    complete={isLoadingComplete}
    onComplete={onCatalogLoadingComplete}
  />
{:else}
  <div
    class="relative flex h-full flex-col overflow-hidden bg-background"
    style="--library-footer-height: clamp(4.5rem, 8vh, 5rem); --app-header-height: clamp(4.25rem, 8vh, 5rem);"
  >
    <header
      class="relative z-20 flex h-[var(--app-header-height)] shrink-0 items-center bg-background px-[clamp(1.5rem,2.5vw,3rem)]"
    >
      <button
        type="button"
        class={`app-chrome-control flex h-9 w-9 items-center justify-center rounded-full border border-border/75 text-muted-foreground transition-[opacity,transform,border-color,color] duration-200 hover:border-foreground/45 hover:text-foreground ${screen.kind !== "home" ? "app-chrome-control--visible" : ""}`}
        aria-label="Back"
        title="Back"
        aria-hidden={screen.kind === "home"}
        tabindex={screen.kind === "home" ? -1 : 0}
        onclick={onBack}
      >
        <ChevronLeft class="h-4 w-4" />
      </button>
      <div class="ml-auto flex items-center gap-2">
        <button
          type="button"
          class={`app-chrome-control flex h-9 w-9 items-center justify-center rounded-full border border-border/75 text-muted-foreground transition-[opacity,transform,border-color,color] duration-200 hover:border-foreground/45 hover:text-foreground ${historyVisible ? "app-chrome-control--visible" : ""}`}
          aria-label="Test history"
          title="Test history"
          aria-hidden={!historyVisible}
          tabindex={historyVisible ? 0 : -1}
          onclick={onOpenHistory}
        >
          <History class="h-4 w-4" />
        </button>
        <button
          type="button"
          class={`app-chrome-control flex h-9 w-9 items-center justify-center rounded-full border border-border/75 text-muted-foreground transition-[opacity,transform,border-color,color] duration-200 hover:border-foreground/45 hover:text-foreground ${screen.kind !== "home" ? "app-chrome-control--visible" : ""}`}
          aria-label="Home"
          title="Home"
          aria-hidden={screen.kind === "home"}
          tabindex={screen.kind === "home" ? -1 : 0}
          onclick={onHome}
        >
          <Home class="h-4 w-4" />
        </button>
        <QuestionSearch
          bind:open={searchOpen}
          sections={searchSections}
          scopeLabel={searchScopeLabel}
          enabled={searchEnabled}
        />
      </div>
    </header>

    <div class="relative z-0 flex min-h-0 flex-1 flex-col overflow-hidden">
      <div class="shrink-0 px-5 pt-8 sm:px-8 sm:pt-10">
        <div class="mx-auto w-full max-w-6xl">
          <div
            class="mb-6 flex min-h-14 items-end justify-center text-center sm:mb-7"
          >
            {#key screenAnimationKey}
              <div class="catalog-view-enter text-center">
                {#if pageTrail}
                  <p
                    class="mb-2 text-[0.7rem] font-bold uppercase tracking-[0.18em] text-muted-foreground/50"
                  >
                    {pageTrail}
                  </p>
                {/if}
                <h1
                  class="text-[1.85rem] font-semibold tracking-[-0.035em] text-foreground sm:text-[2.15rem]"
                >
                  {pageTitle}
                </h1>
              </div>
            {/key}
          </div>
        </div>
      </div>

      <div class="relative min-h-0 flex-1 overflow-hidden">
        <div
          bind:this={scrollElement}
          class={`absolute inset-0 overflow-x-hidden px-5 pb-6 sm:px-8 ${screen.kind === "home" ? "overflow-hidden" : "overflow-y-auto no-scrollbar"}`}
        >
          {#key screenAnimationKey}
            <div
              class={`catalog-view-enter mx-auto h-full w-full max-w-6xl ${screen.kind === "home" ? "" : "catalog-scroll-content"}`}
            >
              <CatalogScreenContent
                {screen}
                {banks}
                {totalQuestions}
                {prelimsCount}
                {mainsCount}
                {mainsPaperTypes}
                {prelimsPapers}
                {mainsPapers}
                {dualPaper1}
                {dualPaper2}
                {isDualPaper}
                {historyEntries}
                {historyLoading}
                {historyLoadingComplete}
                {historyError}
                {onHistoryLoadingComplete}
                {onScreenChange}
                {onOpenHistory}
                {onOpenResult}
                {onOpenPrelim}
                {onOpenTheory}
              />
            </div>
          {/key}
        </div>
        <ScrollIndicator
          scroller={scrollElement}
          updateTrigger={screen}
          trackInsetTop="clamp(0.75rem, 2vh, 1.5rem)"
          trackInsetBottom="clamp(0.75rem, 2vh, 1.5rem)"
        />
      </div>
    </div>

    <footer
      class="pointer-events-auto relative z-20 flex h-[var(--library-footer-height)] shrink-0 items-center border-t border-border/20 bg-transparent px-[clamp(1.5rem,2.5vw,3rem)] py-3"
    >
      <div class="pointer-events-auto flex items-center gap-2">
        <ShortcutsLauncher variant="circle" />
        <OptionalPreferencesModal />
        <ThemeSwitcher direction="right" />
      </div>
    </footer>
  </div>
{/if}
