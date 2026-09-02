import { invoke, isTauri } from "@tauri-apps/api/core";
import { tick } from "svelte";
import { setMode } from "mode-watcher";
import { logError } from "$lib/services/logger";
import { getTheme, initSettings } from "$lib/stores/settings.svelte";
import { withLoadingTimeout } from "$lib/services/loading-timeout";

/** Restore the database preference before the loading screen becomes visible. */
export async function loadStartupTheme(): Promise<void> {
  await withLoadingTimeout(initSettings());
  setMode(getTheme());
}

/** Wait for the themed DOM before exposing the native WebView's first frame. */
export async function revealStartupWindow(): Promise<void> {
  if (!isTauri()) return;
  await tick();
  await new Promise<void>((resolve) => {
    let frame = 0;
    let finished = false;
    const finish = () => {
      if (finished) return;
      finished = true;
      clearTimeout(timer);
      cancelAnimationFrame(frame);
      resolve();
    };
    // Hidden WebViews may suspend animation frames. The DOM/theme is already
    // committed above, so a bounded fallback avoids a hidden-window deadlock.
    const timer = setTimeout(finish, 100);
    frame = requestAnimationFrame(() => {
      frame = requestAnimationFrame(finish);
    });
  });
  try {
    await invoke("startup_ready");
  } catch (cause) {
    // The native watchdog remains available if IPC/bootstrap fails.
    void logError("Could not reveal the startup window", cause);
  }
}
