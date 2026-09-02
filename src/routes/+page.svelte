<!--
  UPSC CSE market-demand home — drill-down catalog.

  Home → Prelims | Mains
  Prelims → GS1 | CSAT → years → Test / Practice
  Mains → Essay | GS1–4 | Mathematics → years (dual: Paper I / Paper II) → theory modal
-->
<script lang="ts">
  import { goto, preloadCode } from "$app/navigation";
  import { onMount } from "svelte";
  import CatalogPageView from "$lib/components/catalog-page-view.svelte";
  import TheoryPaperModal from "$lib/components/theory-paper-modal.svelte";
  import SessionDialogPanel from "$lib/components/session-dialog-panel.svelte";
  import { Dialog } from "$lib/components/ui/dialog";
  import { ACTIVE_UPSC_SECTIONS } from "$lib/constants/upsc-catalog";
  import { SESSION_LOAD_TIMEOUT_MS } from "$lib/constants/timer";
  import {
    catalogHeading,
    catalogReturnTo,
    catalogRouteFromSearchParams,
    banksForSelectedOptionals,
    mainsPaperTypesForOptionals,
    paperItems,
    parseBankMetadata,
    searchScope,
    type CatalogScreen,
    type PaperListItem,
  } from "$lib/services/catalog-model";
  import {
    getQuestionBankWithQuestions,
    getQuestionBanks,
  } from "$lib/services/question-bank";
  import { formatDuration } from "$lib/utils";
  import { logError } from "$lib/services/logger";
  import { getSettings } from "$lib/stores/settings.svelte";
  import { withLoadingTimeout } from "$lib/services/loading-timeout";
  import { isTypingTarget } from "$lib/services/session-keyboard";
  import {
    createTestAttempt,
    listTestAttemptHistory,
  } from "$lib/services/test-session";
  import { seedUpscBanksIfNeeded } from "$lib/services/upsc-seed";
  import type {
    Question,
    StoredQuestionBank,
    TestAttemptHistoryEntry,
    TestMode,
  } from "$lib/types";
  import { toast } from "svelte-sonner";

  let banks = $state<StoredQuestionBank[]>([]);
  let isLoading = $state(true);
  let isLoadingComplete = $state(false);
  let screen = $state<CatalogScreen>({ kind: "home" });
  let screenHistory = $state<CatalogScreen[]>([]);
  let catalogLoadGen = 0;
  let searchOpen = $state(false);
  let historyEntries = $state<TestAttemptHistoryEntry[]>([]);
  let historyLoading = $state(false);
  let historyLoadingComplete = $state(false);
  let historyError = $state<string | null>(null);

  // Prelims start session
  let startDialogOpen = $state(false);
  let selectedBank = $state<StoredQuestionBank | null>(null);
  let selectedMode = $state<TestMode>("practice");
  let isStarting = $state(false);

  // Theory view modal
  let theoryOpen = $state(false);
  let theoryTitle = $state("");
  let theorySubtitle = $state("");
  let theoryPaperCode = $state("");
  let theoryQuestions = $state<Question[]>([]);
  let theoryLoading = $state(false);
  let theoryLoadingComplete = $state(false);
  let theoryError = $state<string | null>(null);

  onMount(() => {
    const handleCatalogBackKey = (event: KeyboardEvent) => {
      if (event.defaultPrevented) return;
      if (
        (event.key !== "Backspace" && event.key !== "Delete") ||
        event.metaKey ||
        event.ctrlKey ||
        event.altKey ||
        isTypingTarget(event.target)
      ) {
        return;
      }

      if (theoryOpen) {
        event.preventDefault();
        theoryOpen = false;
        return;
      }

      if (startDialogOpen) {
        event.preventDefault();
        startDialogOpen = false;
        return;
      }

      if (searchOpen) {
        event.preventDefault();
        return;
      }

      if (screen.kind !== "home") {
        event.preventDefault();
        goBack();
      }
    };

    window.addEventListener("keydown", handleCatalogBackKey);
    void preloadCode("/test/preload");
    void preloadCode("/practice/preload");
    void loadCatalog();
    const restoredRoute = catalogRouteFromSearchParams(
      new URLSearchParams(window.location.search),
    );
    if (restoredRoute) {
      screen = restoredRoute.screen;
      screenHistory = restoredRoute.history;
      if (screen.kind === "prelims-history") void openHistory(false);
    }

    return () => {
      window.removeEventListener("keydown", handleCatalogBackKey);
    };
  });

  async function loadCatalog() {
    const gen = ++catalogLoadGen;
    isLoading = true;
    isLoadingComplete = false;
    let loaded = false;
    try {
      const seedResult = await withLoadingTimeout(seedUpscBanksIfNeeded());
      if (gen !== catalogLoadGen) return;
      if (seedResult.failed > 0) {
        toast.error(`${seedResult.failed} UPSC paper updates failed`);
      }
      banks = (await withLoadingTimeout(getQuestionBanks())).filter((bank) =>
        ACTIVE_UPSC_SECTIONS.has(parseBankMetadata(bank).section),
      );
      if (gen !== catalogLoadGen) return;
      if (banks.length === 0) {
        toast.error("Failed. Try again. Restart if it keeps failing.");
      } else {
        loaded = true;
      }
    } catch (error) {
      if (gen !== catalogLoadGen) return;
      await logError("Failed to load UPSC catalog", error);
      toast.error("Failed. Try again. Restart if it keeps failing.");
    } finally {
      if (gen === catalogLoadGen) {
        if (loaded) isLoadingComplete = true;
        else isLoading = false;
      }
    }
  }

  function finishCatalogLoading() {
    isLoading = false;
    isLoadingComplete = false;
  }

  const settings = $derived(getSettings());
  const visibleCatalogBanks = $derived(
    banksForSelectedOptionals(banks, settings.optionalSubjectIds),
  );
  const visibleMainsPaperTypes = $derived(
    mainsPaperTypesForOptionals(settings.optionalSubjectIds),
  );
  const totalCatalogQuestions = $derived(
    visibleCatalogBanks.reduce((total, bank) => total + bank.totalQuestions, 0),
  );
  const prelimsCatalogPaperCount = $derived(
    visibleCatalogBanks.filter((bank) =>
      parseBankMetadata(bank).section.startsWith("prelims-"),
    ).length,
  );
  const mainsCatalogPaperCount = $derived(
    visibleCatalogBanks.filter((bank) =>
      parseBankMetadata(bank).section.startsWith("mains-"),
    ).length,
  );
  const currentSearchScope = $derived(
    searchScope(
      screen,
      settings.optionalSubjectIds,
      settings.showOptionalResults,
    ),
  );
  const searchSections = $derived(currentSearchScope.sections);
  const searchScopeLabel = $derived(currentSearchScope.label);
  const prelimsPapers = $derived.by((): PaperListItem[] => {
    if (screen.kind !== "prelims-paper") return [];
    return paperItems(visibleCatalogBanks, [screen.paper.section], "prelims");
  });

  const isDualPaper = $derived(
    screen.kind === "mains-paper" && Boolean(screen.paper.dualPaper),
  );

  const mainsListItems = $derived.by((): PaperListItem[] => {
    if (screen.kind !== "mains-paper") return [];
    if (screen.paper.dualPaper) return [];
    return paperItems(visibleCatalogBanks, screen.paper.sections, "theory");
  });

  /** Dual-paper optionals: year tiles under separate Paper I / Paper II headings */
  const dualPaper1Items = $derived.by((): PaperListItem[] => {
    if (!isDualPaper || screen.kind !== "mains-paper") return [];
    const section = screen.paper.sections[0];
    if (!section) return [];
    return paperItems(visibleCatalogBanks, [section], "theory");
  });

  const dualPaper2Items = $derived.by((): PaperListItem[] => {
    if (!isDualPaper || screen.kind !== "mains-paper") return [];
    const section = screen.paper.sections[1];
    if (!section) return [];
    return paperItems(visibleCatalogBanks, [section], "theory");
  });

  function goHome() {
    screen = { kind: "home" };
    screenHistory = [];
  }

  function navigateTo(next: CatalogScreen) {
    screenHistory = [...screenHistory, screen];
    screen = next;
  }

  function goBack() {
    const previous = screenHistory.at(-1);
    if (!previous) return goHome();
    screen = previous;
    screenHistory = screenHistory.slice(0, -1);
  }

  async function openHistory(push = true) {
    if (push) navigateTo({ kind: "prelims-history" });
    historyEntries = [];
    historyLoading = true;
    historyLoadingComplete = false;
    historyError = null;
    let loaded = false;
    try {
      historyEntries = await withLoadingTimeout(listTestAttemptHistory());
      loaded = true;
    } catch (error) {
      await logError("Failed to load test history", error);
      historyError =
        error instanceof Error ? error.message : "Failed to load test history";
    } finally {
      if (loaded) historyLoadingComplete = true;
      else historyLoading = false;
    }
  }

  function finishHistoryLoading() {
    historyLoading = false;
    historyLoadingComplete = false;
  }

  function openPrelimPicker(bank: StoredQuestionBank) {
    selectedBank = bank;
    selectedMode = "practice";
    startDialogOpen = true;
  }

  async function startSelectedMode(mode: TestMode) {
    if (!selectedBank || isStarting) return;
    selectedMode = mode;
    isStarting = true;
    try {
      const bankId = selectedBank.id;
      const attemptId = await Promise.race([
        createTestAttempt(bankId, mode),
        new Promise<never>((_, reject) => {
          window.setTimeout(() => {
            reject(
              new Error(
                "Timed out starting session. Restart the app if UPSC papers are still seeding.",
              ),
            );
          }, SESSION_LOAD_TIMEOUT_MS);
        }),
      ]);
      startDialogOpen = false;
      const returnTo = catalogReturnTo({ history: screenHistory, screen });
      await goto(
        `/${mode}/${attemptId}?returnTo=${encodeURIComponent(returnTo)}`,
      );
    } catch (error) {
      await logError("Failed to start session", error);
      toast.error(
        error instanceof Error
          ? error.message.toUpperCase()
          : "FAILED TO START",
      );
    } finally {
      isStarting = false;
    }
  }

  function paperCodeFromBank(bank: StoredQuestionBank): string {
    try {
      const meta = JSON.parse(bank.metadata) as Record<string, unknown>;
      if (typeof meta.paper === "string") return meta.paper;
      if (typeof meta.section === "string") return meta.section;
    } catch {
      /* ignore */
    }
    return bank.name;
  }

  function formatTheoryTitle(
    bank: StoredQuestionBank,
    meta: Record<string, unknown>,
  ): string {
    const year =
      typeof meta.year === "number" ? meta.year : parseBankMetadata(bank).year;
    const paper =
      typeof meta.paper === "string" ? meta.paper.toUpperCase() : "";

    if (paper === "ESSAY") return `Mains Essay · ${year}`;
    if (/^GS[1-4]$/.test(paper)) return `Mains ${paper} · ${year}`;

    const optionalTitles: Record<string, [string, string]> = {
      MATHS1: ["Mathematics", "I"],
      MATHS2: ["Mathematics", "II"],
    };
    const opt = optionalTitles[paper];
    if (opt) return `${opt[0]} Optional · Paper ${opt[1]} · ${year}`;

    // Fallback: clean stored name
    return bank.name;
  }

  async function openTheoryPaper(item: PaperListItem) {
    theoryTitle = item.bank.name;
    theorySubtitle = "";
    theoryPaperCode = paperCodeFromBank(item.bank);
    theoryQuestions = [];
    theoryError = null;
    theoryLoading = true;
    theoryLoadingComplete = false;
    theoryOpen = true;
    let loaded = false;

    try {
      const payload = await withLoadingTimeout(
        getQuestionBankWithQuestions(item.bank.id),
      );
      if (!payload) {
        theoryError = "Could not load this paper.";
        return;
      }
      theoryQuestions = payload.questions;
      try {
        const meta = JSON.parse(payload.metadata) as Record<string, unknown>;
        if (typeof meta.paper === "string") theoryPaperCode = meta.paper;
        theoryTitle = formatTheoryTitle(item.bank, meta);
      } catch {
        theoryTitle = payload.name;
      }
      theorySubtitle = "";
      loaded = true;
    } catch (error) {
      await logError("Failed to load theory paper", error);
      theoryError =
        error instanceof Error ? error.message : "Failed to load paper";
    } finally {
      if (loaded) theoryLoadingComplete = true;
      else theoryLoading = false;
    }
  }

  function finishTheoryLoading() {
    theoryLoading = false;
    theoryLoadingComplete = false;
  }

  const heading = $derived(catalogHeading(screen));
  const pageTitle = $derived(heading.title);
  const pageTrail = $derived(heading.trail);

  const screenAnimationKey = $derived.by(() => {
    if (screen.kind === "prelims-paper")
      return `${screen.kind}-${screen.paper.id}`;
    if (screen.kind === "mains-paper")
      return `${screen.kind}-${screen.paper.id}`;
    return screen.kind;
  });
</script>

<svelte:head>
  <title>UPSC CSE · PrepLoop</title>
</svelte:head>

<CatalogPageView
  {isLoading}
  {isLoadingComplete}
  {screen}
  banks={visibleCatalogBanks}
  {pageTitle}
  {pageTrail}
  {screenAnimationKey}
  totalQuestions={totalCatalogQuestions}
  prelimsCount={prelimsCatalogPaperCount}
  mainsCount={mainsCatalogPaperCount}
  mainsPaperTypes={visibleMainsPaperTypes}
  {prelimsPapers}
  mainsPapers={mainsListItems}
  dualPaper1={dualPaper1Items}
  dualPaper2={dualPaper2Items}
  {isDualPaper}
  {historyEntries}
  {historyLoading}
  {historyLoadingComplete}
  {historyError}
  bind:searchOpen
  {searchSections}
  {searchScopeLabel}
  searchEnabled={!theoryOpen && !startDialogOpen && searchSections.length > 0}
  onCatalogLoadingComplete={finishCatalogLoading}
  onHistoryLoadingComplete={finishHistoryLoading}
  onBack={goBack}
  onHome={goHome}
  onOpenHistory={() => void openHistory()}
  onScreenChange={navigateTo}
  onOpenResult={(id) =>
    void goto(
      `/results/${id}?returnTo=${encodeURIComponent(
        catalogReturnTo({ history: screenHistory, screen }),
      )}`,
    )}
  onOpenPrelim={openPrelimPicker}
  onOpenTheory={(item) => void openTheoryPaper(item)}
/>

<!-- Prelims: pick Test or Practice after clicking a year row -->
<Dialog bind:open={startDialogOpen}>
  <SessionDialogPanel
    title="READY ?"
    primaryLabel={isStarting && selectedMode === "practice"
      ? "STARTING..."
      : "PRACTICE"}
    secondaryActionLabel={isStarting && selectedMode === "test"
      ? "STARTING..."
      : "TEST"}
    onPrimary={() => void startSelectedMode("practice")}
    onSecondaryAction={() => void startSelectedMode("test")}
    onSecondary={() => (startDialogOpen = false)}
    primaryVariant="outline"
    secondaryActionVariant="default"
    primaryDisabled={isStarting}
    secondaryDisabled={isStarting}
    contentClass="max-w-[27.5rem]"
    bodyClass="space-y-2 px-6 pt-5 pb-3"
    footerClass="px-6 py-3"
  >
    {#if selectedBank}
      <div
        class="text-[1.08rem] font-medium leading-[1.35] tracking-[-0.015em] text-foreground"
      >
        {selectedBank.name}
      </div>
      <div
        class="ui-small-label flex items-center gap-3.5 text-muted-foreground/64"
      >
        <span>{selectedBank.totalQuestions}Q</span>
        <span aria-hidden="true">·</span>
        <span>{formatDuration(selectedBank.defaultDuration)}</span>
      </div>
    {/if}
  </SessionDialogPanel>
</Dialog>

<!-- Mains / theory: full-screen paper viewer (not a dialog) -->
{#if theoryOpen}
  <TheoryPaperModal
    bind:open={theoryOpen}
    title={theoryTitle}
    subtitle={theorySubtitle}
    paperCode={theoryPaperCode}
    questions={theoryQuestions}
    isLoading={theoryLoading}
    loadingComplete={theoryLoadingComplete}
    onLoadingComplete={finishTheoryLoading}
    error={theoryError}
  />
{/if}
