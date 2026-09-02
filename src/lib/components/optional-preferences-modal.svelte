<script lang="ts">
  import { tick } from "svelte";
  import { Heart, Settings2, X } from "@lucide/svelte";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { Button } from "$lib/components/ui/button";
  import {
    Dialog,
    DialogContent,
    DialogTitle,
  } from "$lib/components/ui/dialog";
  import { Switch } from "$lib/components/ui/switch";
  import { MAINS_PAPER_TYPES } from "$lib/constants/upsc-catalog";
  import { getSettings, updateSetting } from "$lib/stores/settings.svelte";
  import { toast } from "svelte-sonner";

  let settingsOpen = $state(false);
  let supportView = $state(false);
  let dialogTitleElement = $state<HTMLElement | null>(null);

  const shortLabels: Record<string, string> = {
    anthropology: "Anthro",
    commerce: "Commerce",
    economics: "Economics",
    geography: "Geography",
    history: "History",
    law: "Law",
    math: "Maths",
    medical: "Medical",
    philosophy: "Philosophy",
    psir: "PSIR",
    pubad: "Pub Ad",
    sociology: "Sociology",
  };
  const optionals = MAINS_PAPER_TYPES.filter((paper) => paper.optional).map(
    (paper) => ({ ...paper, shortLabel: shortLabels[paper.id] ?? paper.label }),
  );
  const settings = $derived(getSettings());
  const selectedOptionalIds = $derived(settings.optionalSubjectIds);
  const showOptionalResults = $derived(settings.showOptionalResults);

  function toggleOptional(subjectId: string, subjectLabel: string) {
    const wasSelected = selectedOptionalIds.includes(subjectId);
    const next = wasSelected
      ? selectedOptionalIds.filter((id) => id !== subjectId)
      : [...selectedOptionalIds, subjectId];
    void updateSetting("optionalSubjectIds", next).then((saved) => {
      if (!saved) return;
      const action = wasSelected ? "REMOVED" : "ADDED";
      toast.success(`${subjectLabel.toUpperCase()} ${action}`, {
        id: `optional-${subjectId}`,
        position: "top-right",
        duration: 1400,
        style: "width: fit-content; min-width: 0; white-space: nowrap;",
      });
    });
  }

  function setShowOptionalResults(checked: boolean) {
    void updateSetting("showOptionalResults", checked);
  }

  async function openGithub() {
    try {
      await openUrl("https://github.com/utilinlabs/preploop");
    } catch {
      toast.error("Could not open GitHub");
    }
  }

  function focusDialogTitle(event: Event) {
    event.preventDefault();
    void tick().then(() => dialogTitleElement?.focus());
  }

  $effect(() => {
    if (!settingsOpen) {
      supportView = false;
    }
  });
</script>

<Button
  variant="ghost"
  size="icon"
  class="h-10 w-10 rounded-full border border-border text-muted-foreground transition-colors hover:border-foreground hover:text-foreground"
  onclick={() => (settingsOpen = true)}
  title="Study preferences"
  aria-label="Study preferences"
  aria-expanded={settingsOpen}
>
  <Settings2 class="h-4 w-4" />
</Button>

<Dialog bind:open={settingsOpen}>
  <DialogContent
    closeOnInteractOutside={true}
    showCloseButton={false}
    onOpenAutoFocus={focusDialogTitle}
    class="h-[29.5rem] max-h-[calc(100dvh-2rem)] w-[calc(100%-2rem)] max-w-md grid-rows-[auto_1px_1fr_auto] gap-0 p-0"
  >
    <div class="flex h-14 items-center justify-between gap-3 px-6">
      <DialogTitle
        bind:ref={dialogTitleElement}
        tabindex={-1}
        class="dialog-title-text outline-none text-[1rem]"
      >
        {supportView ? "SUPPORT PREPLOOP" : "SETTINGS"}
      </DialogTitle>
      <Button
        variant="ghost"
        size="icon-sm"
        class="rounded-full border border-transparent text-muted-foreground/70 transition-colors hover:border-border hover:text-foreground"
        aria-label={supportView ? "Back to settings" : "Close settings"}
        onclick={() => {
          if (supportView) supportView = false;
          else settingsOpen = false;
        }}
      >
        <X class="h-4 w-4" />
      </Button>
    </div>

    <div class="mx-6 h-px bg-border/70"></div>

    {#if supportView}
      <div class="flex h-full flex-col items-center px-6 py-6">
        <p
          class="ui-small-label max-w-xs text-center text-[0.78rem] leading-relaxed text-muted-foreground"
        >
          KEEP PREPLOOP GROWING, ONE LOOP AT A TIME.
        </p>
        <div class="mt-5 w-full max-w-[15.5rem] bg-white p-2.5">
          <img
            src="/upi.png"
            alt="UPI QR code for supporting PrepLoop"
            class="block aspect-square w-full"
          />
        </div>
        <Button
          variant="ghost"
          size="icon"
          class="mt-4 h-10 w-10 rounded-full text-muted-foreground transition-colors hover:bg-transparent hover:text-foreground"
          title="Open PrepLoop on GitHub"
          aria-label="Open PrepLoop on GitHub"
          onclick={() => void openGithub()}
        >
          <svg
            viewBox="0 0 16 16"
            class="h-5 w-5 fill-current"
            aria-hidden="true"
          >
            <path
              d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82A7.65 7.65 0 0 1 8 3.31c.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8"
            />
          </svg>
        </Button>
      </div>
    {:else}
      <div class="overflow-hidden px-6 pt-6">
        <div class="mb-4 flex items-baseline justify-between gap-4">
          <div class="ui-small-label text-[0.78rem] text-muted-foreground/78">
            CHOOSE OPTIONAL
          </div>
          <span
            class="text-[0.61rem] font-bold uppercase tracking-[0.14em] text-muted-foreground/42"
          >
            ONE OR MORE
          </span>
        </div>
        <div class="grid grid-cols-3 gap-2">
          {#each optionals as optional (optional.id)}
            <button
              type="button"
              class={`ui-small-label min-h-11 rounded-none px-2.5 py-2 text-center text-[0.76rem] transition-colors ${selectedOptionalIds.includes(optional.id) ? "bg-foreground text-background" : "bg-muted/45 text-muted-foreground/70 hover:bg-muted hover:text-foreground"}`}
              aria-pressed={selectedOptionalIds.includes(optional.id)}
              aria-label={optional.label}
              title={optional.label}
              onclick={() => toggleOptional(optional.id, optional.shortLabel)}
            >
              {optional.shortLabel}
            </button>
          {/each}
        </div>

        <div
          class="my-[1.0625rem] grid h-12 grid-cols-[minmax(0,1fr)_auto] items-center gap-4 px-1"
        >
          <label
            for="show-optional-results"
            class="ui-small-label leading-none text-[0.78rem] text-muted-foreground/78"
            >SHOW OPTIONALS IN SEARCH</label
          >
          <Switch
            id="show-optional-results"
            size="wide"
            class="self-center rounded-none transition-[background-color,border-color,box-shadow,transform] duration-300 ease-out active:scale-[0.96] motion-reduce:transition-none [&_[data-slot=switch-thumb]]:rounded-none [&_[data-slot=switch-thumb]]:transition-[transform,background-color] [&_[data-slot=switch-thumb]]:duration-300 [&_[data-slot=switch-thumb]]:ease-[cubic-bezier(0.2,0.8,0.2,1)] [&_[data-slot=switch-thumb]]:motion-reduce:transition-none"
            checked={showOptionalResults}
            onCheckedChange={setShowOptionalResults}
          />
        </div>
      </div>

      <div class="px-6">
        <div class="flex justify-center py-3">
          <Button
            variant="ghost"
            size="sm"
            class="ui-small-label rounded-none border border-red-500/45 bg-red-500/8 px-3 text-[0.78rem] text-foreground/90 hover:border-red-500/75 hover:bg-red-500/15 hover:text-foreground"
            onclick={() => (supportView = true)}
          >
            <Heart
              class="relative -top-px h-3.5 w-3.5 fill-red-500 text-red-500"
            />
            Support PrepLoop
          </Button>
        </div>
      </div>
    {/if}
  </DialogContent>
</Dialog>
