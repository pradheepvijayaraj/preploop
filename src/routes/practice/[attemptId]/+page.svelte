<!-- Practice session page — `/practice/[attemptId]`. Untimed session with optional immediate answer feedback. -->
<script lang="ts">
  import { page } from "$app/state";
  import { goto } from "$app/navigation";
  import { onMount, onDestroy } from "svelte";
  import { Button } from "$lib/components/ui/button";
  import { Dialog } from "$lib/components/ui/dialog";
  import LoadingProgress from "$lib/components/loading-progress.svelte";
  import SessionDialogPanel from "$lib/components/session-dialog-panel.svelte";
  import { Switch } from "$lib/components/ui/switch";
  import { Label } from "$lib/components/ui/label";
  import { toast } from "svelte-sonner";
  import {
    getPracticeQuestionFeedback,
    submitTest as submitTestService,
  } from "$lib/services/test-session";
  import type { PracticeQuestionFeedback } from "$lib/types";
  import { logError } from "$lib/services/logger";
  import { safeResultReturnTo } from "$lib/services/result-navigation";
  import {
    createSessionKeyboardHandler,
    getShortcutAnswer,
  } from "$lib/services/session-keyboard";
  import { loadSessionWithTimeout } from "$lib/services/session-loader";
  import {
    getTestSessionState,
    goToQuestion,
    nextQuestion,
    previousQuestion,
    saveAnswer,
    toggleCurrentFlag,
    clearTestSession,
    getNavigationInfo,
    getProgress,
    setSubmitting,
    flushPendingSaves,
    getCurrentQuestion,
    getAnswer,
    isFlagged,
    initTestSession,
  } from "$lib/stores/test-session.svelte";
  import { getSettings, updateSetting } from "$lib/stores/settings.svelte";
  import SessionWorkspace from "$lib/components/session-workspace.svelte";
  import SessionSummaryGrid from "$lib/components/session-summary-grid.svelte";
  import { isUuid } from "$lib/utils";

  const returnTo = $derived(safeResultReturnTo(page.url.searchParams));

  let isLoading = $state(true);
  let isLoadingComplete = $state(false);
  let loadError = $state<string | null>(null);
  let showSubmitDialog = $state(false);
  let navigatorExpanded = $state(false);
  let showFeedback = $state(true);
  let feedbackByQuestion = $state(new Map<string, PracticeQuestionFeedback>());
  const feedbackRequests = new Map<string, Promise<void>>();
  /** Bumped to ignore late load results after cancel / unmount. */
  let loadGeneration = 0;

  // Reactive state from store
  let sessionState = $derived(getTestSessionState());
  let currentQuestion = $derived(getCurrentQuestion());
  let displayedQuestion = $derived.by(() => {
    if (!currentQuestion) return null;
    const feedback = feedbackByQuestion.get(currentQuestion.id);
    return feedback
      ? {
          ...currentQuestion,
          correctAnswers: feedback.correctAnswers,
          explanation: feedback.explanation,
        }
      : currentQuestion;
  });
  let feedbackVisible = $derived(
    showFeedback &&
      currentQuestion !== null &&
      feedbackByQuestion.has(currentQuestion.id),
  );
  let navigation = $derived(getNavigationInfo());
  let progress = $derived(getProgress());
  let settings = $derived(getSettings());

  const handleKeydown = createSessionKeyboardHandler({
    mode: "practice",
    isDialogOpen: () => showSubmitDialog,
    getCurrentQuestion: () => getCurrentQuestion(),
    onNext: nextQuestion,
    onPrevious: previousQuestion,
    onToggleFlag: handleToggleFlag,
    onOpenSubmit: () => {
      showSubmitDialog = true;
    },
    onOptionShortcut: handleOptionShortcut,
  });

  onMount(() => {
    // Read params from the live page store (not a stale $derived capture).
    const id = page.params.attemptId || "";
    if (!id || !isUuid(id)) {
      loadError = "Invalid attempt ID";
      isLoading = false;
      return;
    }

    navigatorExpanded = settings.navigatorExpanded;
    showFeedback = settings.practiceShowImmediateFeedback;
    void loadPracticeSession(id);
    window.addEventListener("keydown", handleKeydown);

    return () => {
      window.removeEventListener("keydown", handleKeydown);
    };
  });

  onDestroy(() => {
    loadGeneration += 1;
    clearTestSession();
  });

  $effect(() => {
    const question = currentQuestion;
    if (
      showFeedback &&
      question &&
      getAnswer(question.id) !== null &&
      !feedbackByQuestion.has(question.id)
    ) {
      void revealFeedback(question.id);
    }
  });

  async function loadPracticeSession(id: string) {
    const gen = ++loadGeneration;
    isLoading = true;
    isLoadingComplete = false;
    loadError = null;
    feedbackByQuestion = new Map();
    feedbackRequests.clear();
    let loaded = false;

    try {
      const result = await loadSessionWithTimeout(id, "practice");
      if (gen !== loadGeneration) return;
      if (result.redirectTo) {
        void goto(
          `${result.redirectTo}?returnTo=${encodeURIComponent(returnTo)}`,
        );
        return;
      }

      if (result.error) {
        loadError = result.error;
        return;
      }
      if (result.data) {
        const { attempt, questions, answers, flags } = result.data;
        initTestSession(
          id,
          attempt.bankId,
          "practice",
          questions,
          attempt.duration,
          0,
          answers,
          flags,
          attempt.status,
        );
        loaded = true;
      }
    } catch (error) {
      if (gen !== loadGeneration) return;
      await logError("Failed to load practice session", error);
      loadError =
        error instanceof Error
          ? error.message
          : "Failed to load practice session";
    } finally {
      if (gen === loadGeneration) {
        if (loaded) isLoadingComplete = true;
        else isLoading = false;
      }
    }
  }

  function finishLoading() {
    isLoading = false;
    isLoadingComplete = false;
  }

  async function handleAnswer(answer: string | string[] | null) {
    const questionId = currentQuestion?.id;
    try {
      await saveAnswer(answer);
    } catch {
      toast.error(
        "Answer could not be saved. Your previous answer was restored.",
      );
      return;
    }
    if (answer !== null && showFeedback && questionId) {
      void revealFeedback(questionId);
    } else if (answer === null && questionId) {
      const next = new Map(feedbackByQuestion);
      next.delete(questionId);
      feedbackByQuestion = next;
    }
  }

  function revealFeedback(questionId: string): Promise<void> {
    if (feedbackByQuestion.has(questionId)) return Promise.resolve();
    const existing = feedbackRequests.get(questionId);
    if (existing) return existing;
    const attemptId = sessionState.attemptId;
    if (!attemptId) return Promise.resolve();

    const request = getPracticeQuestionFeedback(attemptId, questionId)
      .then((feedback) => {
        if (sessionState.attemptId !== attemptId) return;
        const next = new Map(feedbackByQuestion);
        next.set(questionId, feedback);
        feedbackByQuestion = next;
      })
      .catch(async (error) => {
        await logError("Failed to load practice feedback", error);
        toast.error("Answer feedback could not be loaded.");
      })
      .finally(() => feedbackRequests.delete(questionId));
    feedbackRequests.set(questionId, request);
    return request;
  }

  function handleOptionShortcut(optionId: string) {
    if (!currentQuestion) return;
    void handleAnswer(
      getShortcutAnswer(
        currentQuestion,
        getAnswer(currentQuestion.id),
        optionId,
      ),
    );
  }

  async function handleToggleFlag() {
    try {
      await toggleCurrentFlag();
    } catch {
      toast.error("Flag could not be updated.");
    }
  }

  async function handleSubmit() {
    // Guard against double-submit
    if (!sessionState.attemptId || sessionState.isSubmitting) return;

    const submittedAttemptId = sessionState.attemptId;
    setSubmitting(true);
    showSubmitDialog = false;

    try {
      await flushPendingSaves();
      await submitTestService(submittedAttemptId);
      clearTestSession(false);
      toast.success("Practice session completed!");
      goto(
        `/results/${submittedAttemptId}?returnTo=${encodeURIComponent(returnTo)}`,
      );
    } catch (error) {
      await logError("Failed to submit practice session", error);
      toast.error("Failed to finish practice session");
      setSubmitting(false);
    }
  }

  function toggleNavigator() {
    navigatorExpanded = !navigatorExpanded;
    updateSetting("navigatorExpanded", navigatorExpanded);
  }

  function toggleFeedback() {
    showFeedback = !showFeedback;
    updateSetting("practiceShowImmediateFeedback", showFeedback);
  }
</script>

<svelte:head>
  <title>Practice - PrepLoop</title>
</svelte:head>

{#if isLoading}
  <LoadingProgress
    class="h-full bg-background"
    complete={isLoadingComplete}
    onComplete={finishLoading}
  />
{:else if loadError}
  <div class="flex h-full items-center justify-center">
    <div class="text-center">
      <h1 class="text-2xl font-bold text-destructive mb-4">Error</h1>
      <p class="text-muted-foreground mb-4">{loadError}</p>
      <div class="flex items-center justify-center gap-3">
        <Button href="/">Back to Dashboard</Button>
        <Button
          variant="outline"
          onclick={() => {
            const id = page.params.attemptId || "";
            if (id && isUuid(id)) void loadPracticeSession(id);
          }}
        >
          Retry
        </Button>
      </div>
    </div>
  </div>
{:else if sessionState.attemptId}
  <SessionWorkspace
    modeLabel="Practice"
    exitLabel="Exit practice"
    session={sessionState}
    question={displayedQuestion}
    answer={currentQuestion ? getAnswer(currentQuestion.id) : null}
    flagged={currentQuestion ? isFlagged(currentQuestion.id) : false}
    {navigation}
    {progress}
    {navigatorExpanded}
    showFeedback={feedbackVisible}
    allowTextSelection={true}
    showTags={true}
    onAnswer={(answer) => void handleAnswer(answer)}
    onToggleFlag={() => void handleToggleFlag()}
    onNavigate={goToQuestion}
    onToggleNavigator={toggleNavigator}
    onPrevious={previousQuestion}
    onNext={nextQuestion}
    onSubmit={() => (showSubmitDialog = true)}
  >
    {#snippet headerCenter()}
      <div class="flex items-center gap-2">
        <Label
          for="feedback-toggle"
          class="cursor-pointer text-xs font-bold uppercase tracking-widest text-muted-foreground/75"
        >
          {showFeedback ? "Answers On" : "Answers Off"}
        </Label>
        <Switch
          id="feedback-toggle"
          checked={showFeedback}
          onCheckedChange={toggleFeedback}
          class="scale-75"
        />
      </div>
    {/snippet}
  </SessionWorkspace>

  <!-- Finish Confirmation Dialog -->
  <Dialog bind:open={showSubmitDialog}>
    <SessionDialogPanel
      title="SUBMIT PRACTICE"
      primaryLabel={sessionState.isSubmitting ? "SUBMITTING..." : "SUBMIT"}
      initialFocus="primary"
      onPrimary={handleSubmit}
      onSecondary={() => (showSubmitDialog = false)}
      primaryDisabled={sessionState.isSubmitting}
      contentClass="max-w-[25rem]"
      headerClass="h-14 px-6"
      dividerClass="mx-6"
      bodyClass="px-6 pt-4 pb-2"
      footerClass="px-6 py-3"
    >
      <SessionSummaryGrid
        answered={progress.answered}
        total={progress.total}
        flagged={progress.flagged}
      />
    </SessionDialogPanel>
  </Dialog>
{/if}
