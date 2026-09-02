<script lang="ts">
  import { onDestroy, onMount, tick } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { getVersion } from "@tauri-apps/api/app";
  import { Channel, invoke, isTauri } from "@tauri-apps/api/core";
  import { check, type Update } from "@tauri-apps/plugin-updater";
  import { relaunch } from "@tauri-apps/plugin-process";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { X, LoaderCircle } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import ScrollIndicator from "$lib/components/scroll-indicator.svelte";
  import { logError } from "$lib/services/logger";
  import {
    Dialog,
    DialogContent,
    DialogTitle,
  } from "$lib/components/ui/dialog";

  type PendingUpdate = {
    currentVersion: string;
    version: string;
    body?: string | null;
    size: number;
    sha256: string;
  };
  type DownloadEvent =
    | {
        event: "Started";
        data: { contentLength?: number | null };
      }
    | {
        event: "Progress";
        data: { chunkLength: number };
      }
    | { event: "Finished" };
  type UpdatePhase =
    | "idle"
    | "checking"
    | "available"
    | "current"
    | "downloading"
    | "ready"
    | "installing"
    | "installed"
    | "error";

  let { home = true } = $props<{ home?: boolean }>();
  let open = $state(false);
  let manualCheck = $state(false);
  let titleElement = $state<HTMLElement | null>(null);
  let releaseNotesElement = $state<HTMLElement | null>(null);
  let notified = false;
  let canInstall = true;

  function focusTitle(event: Event) {
    event.preventDefault();
    void tick().then(() => titleElement?.focus());
  }

  function compactReleaseNotes(value: string) {
    return value.trim().replace(/\n[ \t]*\n+/g, "\n");
  }

  $effect(() => {
    if (home && phase === "idle") void checkForUpdate();
    if (!home && !manualCheck) open = false;
  });
  let phase = $state<UpdatePhase>("idle");
  let currentVersion = $state("");
  let nextVersion = $state("");
  let notes = $state("");
  let error = $state("");
  let errorDetail = $state("");
  let received = $state(0);
  let total = $state(0);
  let update: Update | null = null;
  let pendingUpdate = $state<PendingUpdate | null>(null);
  let disposed = false;
  let checkGeneration = 0;
  let checkTimer: ReturnType<typeof setTimeout> | undefined;
  let noticeTimer: ReturnType<typeof setTimeout> | undefined;
  const updateCheckTimeout = 35_000;
  const updateNoticeDelay = 650;
  const busy = $derived(
    ["checking", "downloading", "installing"].includes(phase),
  );

  const downloadPercent = $derived(
    total > 0 ? Math.min(100, Math.round((received / total) * 100)) : undefined,
  );
  const cornerLabel = $derived(
    phase === "ready"
      ? "Install update"
      : phase === "installed"
        ? "Restarting PrepLoop"
        : phase === "installing"
          ? "Installing update"
          : phase === "error"
            ? "Retry update"
            : "Update available",
  );

  function releaseUpdate(resource: Update | null) {
    void resource
      ?.close()
      .catch((cause) => logError("Could not release updater resource", cause));
  }

  async function checkForUpdate() {
    if (busy || disposed) return;
    const generation = ++checkGeneration;
    const active = () => !disposed && generation === checkGeneration;
    phase = "checking";
    error = "";
    errorDetail = "";
    nextVersion = "";
    notes = "";
    releaseUpdate(update);
    update = null;
    let stage = "reading the installed version";
    // The UI deadline also covers stalled IPC before the HTTP request starts.
    // It does not cancel native IPC: late results must be discarded/released.
    const timer = setTimeout(() => {
      if (!active()) return;
      checkGeneration++;
      error = "SOMETHING WENT WRONG";
      errorDetail = "INSTALL LATEST VERSION FROM GITHUB";
      phase = "error";
      void logError(`Update check timed out while ${stage}`);
    }, updateCheckTimeout);
    checkTimer = timer;
    try {
      if (!isTauri()) {
        error = "SOMETHING WENT WRONG";
        errorDetail = "INSTALL LATEST VERSION FROM GITHUB";
        phase = "error";
        return;
      }
      const version = await getVersion();
      if (!active()) return;
      currentVersion = version;
      stage = "checking package support";
      const supported = await invoke<boolean>("supports_in_app_updates");
      if (!active()) return;
      canInstall = supported;
      if (!canInstall) {
        error = "SOMETHING WENT WRONG";
        errorDetail = "INSTALL LATEST VERSION FROM GITHUB";
      }
      const storedPending = await invoke<PendingUpdate | null>(
        "get_pending_update",
        { currentVersion: version },
      );
      if (!active()) return;
      pendingUpdate = storedPending;
      stage = "fetching release information from GitHub";
      const found = await check({ timeout: 30_000 });
      if (!active()) {
        releaseUpdate(found);
        return;
      }
      update = found;
      nextVersion = found?.version ?? storedPending?.version ?? "";
      notes = compactReleaseNotes(found?.body ?? storedPending?.body ?? "");
      phase = found
        ? storedPending?.version === found.version && canInstall
          ? "ready"
          : canInstall
            ? "available"
            : "error"
        : storedPending && canInstall
          ? "ready"
          : "current";
      if (
        found &&
        home &&
        !notified &&
        !document.querySelector('[role="dialog"]')
      ) {
        notified = true;
        if (manualCheck) {
          open = true;
        } else {
          noticeTimer = setTimeout(() => {
            noticeTimer = undefined;
            if (!disposed && home && nextVersion && phase !== "downloading")
              open = true;
          }, updateNoticeDelay);
        }
      }
    } catch (cause) {
      if (!active()) return;
      if (pendingUpdate && canInstall) {
        nextVersion = pendingUpdate.version;
        notes = compactReleaseNotes(pendingUpdate.body ?? "");
        phase = "ready";
        return;
      }
      error = "SOMETHING WENT WRONG";
      errorDetail = "INSTALL LATEST VERSION FROM GITHUB";
      phase = "error";
      void logError(`Update check failed while ${stage}`, cause);
    } finally {
      clearTimeout(timer);
      if (checkTimer === timer) checkTimer = undefined;
    }
  }

  async function showDownloadResult() {
    await tick();
    const dialog = document.querySelector('[role="dialog"]');
    if (
      (home || manualCheck) &&
      (!dialog || (titleElement && dialog.contains(titleElement)))
    )
      open = true;
  }

  async function downloadUpdate() {
    if (!update || phase !== "available") return;
    phase = "downloading";
    open = false;
    received = 0;
    total = 0;
    try {
      const channel = new Channel<DownloadEvent>();
      channel.onmessage = (event) => {
        if (event.event === "Started") total = event.data.contentLength ?? 0;
        if (event.event === "Progress") received += event.data.chunkLength;
      };
      pendingUpdate = await invoke<PendingUpdate>("download_pending_update", {
        expectedVersion: update.version,
        onEvent: channel,
      });
      if (!disposed) {
        phase = "ready";
        await showDownloadResult();
      }
    } catch (cause) {
      if (disposed) return;
      error = "SOMETHING WENT WRONG";
      errorDetail = "INSTALL LATEST VERSION FROM GITHUB";
      phase = "error";
      void logError("Update download failed", cause);
      await showDownloadResult();
    }
  }

  async function restart() {
    if (!home) return;
    try {
      await relaunch();
    } catch (cause) {
      error = "SOMETHING WENT WRONG";
      errorDetail = "INSTALL LATEST VERSION FROM GITHUB";
      phase = "error";
      open = true;
      void logError("Could not restart after installing the update", cause);
    }
  }

  async function installUpdate() {
    if (!home || phase !== "ready") return;
    const version = pendingUpdate?.version ?? update?.version;
    if (!version) return;
    phase = "installing";
    open = false;
    try {
      await invoke("install_pending_update", { expectedVersion: version });
    } catch (cause) {
      error = "SOMETHING WENT WRONG";
      errorDetail = "INSTALL LATEST VERSION FROM GITHUB";
      phase = "error";
      open = true;
      void logError("Update installation failed", cause);
      return;
    }
    phase = "installed";
    await restart();
  }

  async function openReleases() {
    try {
      await openUrl("https://github.com/utilinlabs/preploop/releases/latest");
    } catch {
      error = "SOMETHING WENT WRONG";
      errorDetail = "INSTALL LATEST VERSION FROM GITHUB";
    }
  }

  onMount(() => {
    if (!isTauri()) return;
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void listen("check-for-updates", () => {
      if (phase === "downloading") return;
      manualCheck = true;
      open = true;
      if (!busy && phase !== "ready" && phase !== "installed")
        void checkForUpdate();
    })
      .then((stop) => {
        if (cancelled) stop();
        else unlisten = stop;
      })
      .catch((cause) =>
        logError("Could not listen for the update menu", cause),
      );
    return () => {
      cancelled = true;
      unlisten?.();
    };
  });

  onDestroy(() => {
    disposed = true;
    checkGeneration++;
    clearTimeout(checkTimer);
    clearTimeout(noticeTimer);
    releaseUpdate(update);
  });
</script>

{#if home && nextVersion && phase !== "installing" && phase !== "installed"}
  {#if phase === "downloading"}
    <div
      class="update-control download-progress"
      role="progressbar"
      aria-label="Update download progress"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={downloadPercent}
      aria-valuetext={downloadPercent === undefined
        ? "Downloading — size unknown"
        : `${downloadPercent}% downloaded`}
      title={downloadPercent === undefined
        ? "Downloading — size unknown"
        : `Downloading ${downloadPercent}%`}
    >
      <span
        class="download-fill"
        style:width={`${downloadPercent ?? 0}%`}
        aria-hidden="true"
      ></span>
      <span class="download-percent" aria-hidden="true"
        >{downloadPercent === undefined ? "…" : `${downloadPercent}%`}</span
      >
    </div>
  {:else}
    <button
      type="button"
      class={`update-control update-trigger ${phase === "ready" ? "update-trigger--ready" : ""}`}
      aria-label={cornerLabel}
      title={cornerLabel}
      onclick={() => (open = true)}
    >
      {cornerLabel}
    </button>
  {/if}
{/if}
{#if open && (home || manualCheck) && phase !== "downloading" && phase !== "installing" && phase !== "installed"}
  <Dialog bind:open>
    <DialogContent
      showCloseButton={false}
      closeOnInteractOutside={!busy}
      onOpenAutoFocus={focusTitle}
      class="w-[calc(100%-2rem)] max-w-[22rem] gap-0 overflow-hidden p-0"
    >
      <div class="flex h-14 items-center justify-between gap-3 px-5">
        <DialogTitle
          bind:ref={titleElement}
          tabindex={-1}
          class="dialog-title-text outline-none"
        >
          {phase === "current"
            ? "YOU’RE UP TO DATE"
            : phase === "checking"
              ? "CHECKING FOR UPDATES"
              : phase === "error"
                ? "UPDATE ERROR"
                : phase === "ready"
                  ? "READY TO INSTALL"
                  : nextVersion
                    ? "UPDATE AVAILABLE"
                    : "UPDATES"}
        </DialogTitle>
        <Button
          variant="ghost"
          size="icon-sm"
          class="rounded-full border border-transparent text-muted-foreground/70 hover:border-border hover:text-foreground"
          aria-label="Close update dialog"
          onclick={() => (open = false)}
        >
          <X class="h-4 w-4" />
        </Button>
      </div>
      <div class="mx-5 h-px bg-border/70"></div>
      <div class={`${phase === "error" ? "space-y-3" : "space-y-5"} px-5 py-5`}>
        {#if phase !== "error" && phase !== "checking" && (nextVersion || phase === "current")}
          <div
            class="grid grid-cols-[1fr_auto_1fr] grid-rows-[auto_auto] items-center"
          >
            <p
              class="col-start-1 row-start-1 px-3 text-center ui-small-label text-[0.65rem] text-muted-foreground/80"
            >
              INSTALLED
            </p>
            <p
              class="col-start-1 row-start-2 mt-1 px-3 text-center text-2xl font-semibold tracking-tight text-muted-foreground"
            >
              {currentVersion}
            </p>
            <span
              class="col-start-2 row-start-2 px-2 text-lg font-medium leading-none text-muted-foreground/45"
              aria-hidden="true">{phase === "current" ? "=" : "->"}</span
            >
            <p
              class="col-start-3 row-start-1 px-3 text-center ui-small-label text-[0.65rem] text-muted-foreground/80"
            >
              LATEST
            </p>
            <p
              class="col-start-3 row-start-2 mt-1 px-3 text-center text-2xl font-semibold tracking-tight"
            >
              {phase === "current" ? currentVersion : nextVersion}
            </p>
          </div>
        {/if}
        {#if phase === "checking"}
          <div
            role="status"
            aria-live="polite"
            class="ui-small-label text-muted-foreground"
          >
            <div class="flex justify-center py-3">
              <LoaderCircle
                class="h-5 w-5 motion-safe:animate-spin"
                aria-hidden="true"
              /><span class="sr-only">CHECKING FOR UPDATES</span>
            </div>
          </div>
        {/if}

        {#if notes && phase === "available"}
          <div class="border-t border-border/60 pt-5">
            <p
              class="ui-small-label mb-2 text-[0.65rem] text-muted-foreground/80"
            >
              WHAT’S NEW
            </p>
            <div class="relative overflow-hidden">
              <div
                bind:this={releaseNotesElement}
                class="max-h-36 overflow-y-auto pr-3 text-sm leading-normal text-muted-foreground no-scrollbar"
              >
                <div class="space-y-1">
                  {#each notes.split("\n") as note}
                    <p>{note}</p>
                  {/each}
                </div>
              </div>
              <ScrollIndicator
                scroller={releaseNotesElement}
                right={0}
                insetY={0}
                updateTrigger={notes}
              />
            </div>
          </div>
        {/if}
        {#if !home && phase === "ready"}
          <p class="ui-small-label text-muted-foreground">
            RETURN HOME TO FINISH
          </p>
        {/if}
        {#if error}<p role="alert" class="ui-small-label text-destructive">
            {error}
          </p>{/if}
        {#if errorDetail}
          <p class="text-xs leading-relaxed text-muted-foreground">
            {errorDetail}
          </p>
        {/if}
      </div>
      {#if phase !== "current" && phase !== "checking"}
        <div class="mx-5 h-px bg-border/70"></div>
        <div class="mx-5 flex items-center justify-between gap-3 py-3">
          {#if phase !== "error"}
            <Button
              variant="ghost"
              class="h-8 px-2 text-[0.65rem] font-semibold uppercase tracking-[0.14em] text-muted-foreground"
              onclick={() => (open = false)}
              >{nextVersion ? "LATER" : "CLOSE"}</Button
            >
          {:else}
            <Button
              variant="ghost"
              class="h-8 px-2 text-[0.65rem] font-semibold uppercase tracking-[0.14em] text-muted-foreground"
              onclick={() => void openReleases()}>OPEN GITHUB</Button
            >
          {/if}
          {#if phase === "available"}
            <Button
              class="ml-auto h-8 px-3 text-[0.65rem] font-semibold uppercase tracking-[0.14em]"
              onclick={() => void downloadUpdate()}>DOWNLOAD</Button
            >
          {:else if phase === "ready"}
            <Button
              class="ml-auto h-8 px-3 text-[0.65rem] font-semibold uppercase tracking-[0.14em]"
              disabled={!home}
              onclick={() => void installUpdate()}>INSTALL NOW</Button
            >
          {:else if phase === "error"}
            <Button
              class="ml-auto h-8 px-3 text-[0.65rem] font-semibold uppercase tracking-[0.14em]"
              onclick={() => void checkForUpdate()}>TRY AGAIN</Button
            >
          {:else}
            <Button
              disabled
              class="ml-auto h-8 px-3 text-[0.65rem] font-semibold uppercase tracking-[0.14em]"
              >CHECKING</Button
            >
          {/if}
        </div>
      {/if}
    </DialogContent>
  </Dialog>
{/if}

<style>
  .update-control {
    --update-surface: #fff;
    --update-foreground: #0a0a0a;
    --update-border: #0a0a0a;
    position: relative;
    display: grid;
    place-items: center;
    width: 160px;
    height: 36px;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    padding: 0;
    overflow: hidden;
    isolation: isolate;
    border: 1px solid var(--update-border);
    border-radius: 0;
    background: var(--update-surface);
    color: var(--update-foreground);
    transition:
      background-color 140ms ease,
      border-color 140ms ease,
      color 140ms ease,
      box-shadow 140ms ease,
      transform 140ms ease;
  }
  .update-trigger {
    cursor: pointer;
    animation: update-attention 2.4s ease-in-out infinite;
    box-shadow: 0 8px 24px
      color-mix(in srgb, var(--update-border) 24%, transparent);
  }
  .update-trigger:hover {
    border-color: var(--update-border);
    background: var(--update-surface);
    color: var(--update-foreground);
    box-shadow: 0 10px 26px
      color-mix(in srgb, var(--update-border) 28%, transparent);
    transform: translateY(-1px);
  }
  .update-trigger--ready {
    --update-surface: #0a0a0a;
    --update-foreground: #fff;
    --update-border: #0a0a0a;
  }
  .update-trigger:focus-visible {
    outline: 2px solid var(--foreground);
    outline-offset: 3px;
  }
  .download-progress {
    --update-surface: #fff;
    --update-foreground: #0a0a0a;
    --update-border: #0a0a0a;
    animation: none;
    box-shadow: none;
  }
  .download-fill {
    position: absolute;
    inset: 0 auto 0 0;
    background: var(--update-foreground);
    transition: width 160ms linear;
  }
  .download-percent {
    position: relative;
    color: #fff;
    mix-blend-mode: difference;
    font-size: 12px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  :global(.dark) .update-control {
    --update-surface: #0a0a0a;
    --update-foreground: #fff;
    --update-border: #fff;
  }
  :global(.dark) .update-trigger--ready {
    --update-surface: #fff;
    --update-foreground: #0a0a0a;
    --update-border: #fff;
  }
  :global(.dark) .download-progress {
    --update-surface: #0a0a0a;
    --update-foreground: #fff;
    --update-border: #fff;
  }
  @keyframes update-attention {
    0%,
    100% {
      box-shadow: 0 8px 24px
        color-mix(in srgb, var(--update-border) 24%, transparent);
    }
    50% {
      box-shadow:
        0 0 0 5px color-mix(in srgb, var(--update-border) 16%, transparent),
        0 14px 32px color-mix(in srgb, var(--update-border) 32%, transparent);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .update-trigger,
    .download-fill {
      transition: none;
      animation: none;
    }
  }
</style>
