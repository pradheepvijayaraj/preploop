<script lang="ts">
  import { onMount } from "svelte";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { toast } from "svelte-sonner";
  import { Button } from "$lib/components/ui/button";
  import { Switch } from "$lib/components/ui/switch";
  import { MAINS_PAPER_TYPES } from "$lib/constants/upsc-catalog";
  import { completeOnboarding, getSettings } from "$lib/stores/settings.svelte";

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
  let selectedIds = $state(
    getSettings().optionalSubjectIds.filter((id) =>
      optionals.some((paper) => paper.id === id),
    ),
  );
  let showOptionalResults = $state(getSettings().showOptionalResults);
  let saving = $state(false);
  let error = $state("");
  let heading: HTMLHeadingElement;

  onMount(() => heading.focus({ preventScroll: true }));

  function toggleOptional(id: string) {
    if (saving) return;
    selectedIds = selectedIds.includes(id)
      ? selectedIds.filter((selected) => selected !== id)
      : [...selectedIds, id];
    error = "";
  }

  async function openRepository(event: MouseEvent) {
    event.preventDefault();
    try {
      await openUrl("https://github.com/utilinlabs/preploop");
    } catch {
      toast.error("Could not open GitHub");
    }
  }

  async function continueSetup(event: SubmitEvent) {
    event.preventDefault();
    if (saving) return;
    saving = true;
    error = "";
    try {
      const saved = await completeOnboarding(
        [...selectedIds],
        showOptionalResults,
      );
      if (!saved) error = "Couldn’t save your preferences. Please try again.";
    } catch {
      error = "Couldn’t save your preferences. Please try again.";
    } finally {
      saving = false;
    }
  }
</script>

<section
  aria-labelledby="welcome-heading"
  class="grid h-full min-h-0 overflow-y-auto bg-background lg:grid-cols-[2fr_3fr]"
>
  <header
    class="flex min-h-[40rem] items-start bg-muted/35 px-[clamp(2.5rem,5vw,5rem)] py-[clamp(6rem,12vh,9rem)] dark:bg-muted/15"
  >
    <div class="w-full max-w-md">
      <h1
        id="welcome-heading"
        bind:this={heading}
        tabindex="-1"
        aria-label="MAKE PREPLOOP YOURS"
        class="dialog-title-text -ml-[clamp(0.75rem,1.5vw,1.5rem)] text-[clamp(2.75rem,4.2vw,3.75rem)] leading-[1.1] tracking-[0.035em] outline-none"
      >
        <span aria-hidden="true" class="flex items-center whitespace-nowrap">
          <span class="mr-[0.22em] text-muted-foreground/76">MAKE</span>
          <span class="text-foreground">PREPL</span>
          <svg
            viewBox="2 7 20 10"
            class="h-[0.8em] w-[1.52em] shrink-0 text-foreground"
            role="presentation"
          >
            <path
              fill="currentColor"
              d="M20.288 9.463a4.856 4.856 0 0 0-4.336-2.3 4.586 4.586 0 0 0-3.343 1.767c.071.116.148.226.212.347l.879 1.652.134-.254a2.71 2.71 0 0 1 2.206-1.519 2.845 2.845 0 1 1 0 5.686 2.708 2.708 0 0 1-2.205-1.518L13.131 12l-1.193-2.26a4.709 4.709 0 0 0-3.89-2.581 4.845 4.845 0 1 0 0 9.682 4.586 4.586 0 0 0 3.343-1.767c-.071-.116-.148-.226-.212-.347l-.879-1.656-.134.254a2.71 2.71 0 0 1-2.206 1.519 2.855 2.855 0 0 1-2.559-1.369 2.825 2.825 0 0 1 0-2.946 2.862 2.862 0 0 1 2.442-1.374h.121a2.708 2.708 0 0 1 2.205 1.518l.7 1.327 1.193 2.26a4.709 4.709 0 0 0 3.89 2.581h.209a4.846 4.846 0 0 0 4.127-7.378z"
            />
          </svg>
          <span class="text-foreground">P</span>
        </span>
        <span class="block text-muted-foreground/76">YOURS</span>
      </h1>
      <div class="mt-[clamp(4rem,8vh,5.5rem)] ml-4">
        <div class="flex flex-col gap-4">
          {#each ["GROWING PYQ LIBRARY", "CONTEXTUAL SEARCH", "FOCUSED PRACTICE", "TIMED TESTS", "LOCAL FIRST BY DESIGN", "NO FUSS"] as feature}
            <div class="flex items-baseline gap-4">
              <span class="text-sm text-muted-foreground/50">—</span>
              <span
                class="ui-small-label text-[0.9rem] leading-relaxed tracking-[0.065em] text-muted-foreground/88"
                >{feature}</span
              >
            </div>
          {/each}
          <div class="flex items-baseline gap-4">
            <span class="text-sm text-muted-foreground/50">—</span>
            <a
              href="https://github.com/utilinlabs/preploop"
              target="_blank"
              rel="noreferrer"
              aria-label="Open PrepLoop on GitHub"
              title="Open PrepLoop on GitHub"
              onclick={openRepository}
              class="ui-small-label inline-flex cursor-pointer items-center gap-2 text-[0.9rem] leading-relaxed tracking-[0.065em] text-foreground/90 underline decoration-foreground/45 decoration-1 underline-offset-4 transition-colors hover:text-foreground hover:decoration-foreground focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-ring"
            >
              OPEN SOURCE
              <svg
                viewBox="0 0 16 16"
                class="h-4 w-4 fill-current"
                aria-hidden="true"
              >
                <path
                  d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82A7.65 7.65 0 0 1 8 3.31c.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8"
                />
              </svg>
            </a>
          </div>
        </div>
        <p
          class="ui-small-label mt-7 text-[0.78rem] tracking-[0.09em] text-muted-foreground/60"
        >
          MORE ON THE WAY...
        </p>
      </div>
    </div>
  </header>

  <div
    class="flex min-h-[32rem] items-center justify-center px-[clamp(2.5rem,5vw,5rem)] py-10"
  >
    <form class="w-full max-w-xl" onsubmit={continueSetup} aria-busy={saving}>
      <div class="mb-5">
        <h2
          class="dialog-title-text mb-6 text-[clamp(1.85rem,2.6vw,2.5rem)] leading-none tracking-[0.1em] text-foreground"
        >
          UPSC CSE
        </h2>
        <div class="ui-small-label text-[0.95rem] text-foreground/90">
          CHOOSE OPTIONAL
        </div>
      </div>
      <fieldset disabled={saving} aria-describedby="welcome-settings-note">
        <legend class="sr-only">Choose optional subjects</legend>
        <div class="grid grid-cols-3 gap-2">
          {#each optionals as optional (optional.id)}
            <button
              type="button"
              aria-pressed={selectedIds.includes(optional.id)}
              aria-label={optional.label}
              title={optional.label}
              onclick={() => toggleOptional(optional.id)}
              class={`ui-small-label min-h-14 rounded-none px-3 py-2.5 text-center text-[0.8rem] transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring disabled:cursor-wait ${selectedIds.includes(optional.id) ? "bg-foreground text-background" : "bg-muted/45 text-muted-foreground/70 hover:bg-muted hover:text-foreground"}`}
            >
              {optional.shortLabel}
            </button>
          {/each}
        </div>

        <div
          class="mt-5 grid h-11 grid-cols-[minmax(0,1fr)_auto] items-center gap-4"
        >
          <label
            for="welcome-optional-results"
            class="ui-small-label text-[0.8rem] leading-none text-foreground/90"
            >SHOW OPTIONALS IN SEARCH</label
          >
          <Switch
            id="welcome-optional-results"
            size="wide"
            class="self-center rounded-none transition-[background-color,border-color,box-shadow,transform] duration-300 ease-out motion-reduce:transition-none [&_[data-slot=switch-thumb]]:rounded-none [&_[data-slot=switch-thumb]]:transition-[transform,background-color] [&_[data-slot=switch-thumb]]:duration-300 [&_[data-slot=switch-thumb]]:ease-[cubic-bezier(0.2,0.8,0.2,1)] [&_[data-slot=switch-thumb]]:motion-reduce:transition-none"
            checked={showOptionalResults}
            onCheckedChange={(checked) => (showOptionalResults = checked)}
            disabled={saving}
          />
        </div>
      </fieldset>

      {#if error}<p role="alert" class="mb-4 text-sm text-destructive">
          {error}
        </p>{/if}
      <div class="mt-6 flex items-center justify-between gap-6">
        <p
          id="welcome-settings-note"
          class="ui-small-label whitespace-nowrap text-[0.75rem] font-medium leading-relaxed tracking-[0.06em] text-muted-foreground/55"
        >
          CHANGE THESE ANYTIME IN SETTINGS
        </p>
        <Button
          type="submit"
          disabled={saving}
          class="ui-small-label h-11 min-w-40 rounded-none bg-foreground px-8 text-[0.8rem] text-background hover:bg-foreground/85"
        >
          {saving ? "Saving…" : "Continue"}
        </Button>
      </div>
    </form>
  </div>
</section>
