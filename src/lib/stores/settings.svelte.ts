/**
 * Settings store (Svelte 5 runes).
 *
 * Manages user preferences (theme, navigator state, etc.) with:
 *   - One-time async initialisation from the backend (with promise lock)
 *   - Optimistic UI updates with conditional rollback on save failure
 *   - Reactive getters via `$state` runes
 *
 * Settings are loaded once at app startup (from +layout.svelte) and
 * persisted incrementally via `updateSetting`.
 */
import { type Settings, DEFAULT_SETTINGS } from "$lib/types";
import { loadSettings, saveSettings } from "$lib/services/settings-persistence";
import { logError } from "$lib/services/logger";
import { MAINS_PAPER_TYPES } from "$lib/constants/upsc-catalog";

// Settings state using Svelte 5 runes
let settings = $state<Settings>({ ...DEFAULT_SETTINGS });
let isLoaded = $state(false);
let persistedSettings: Settings = { ...DEFAULT_SETTINGS };

// Promise to track ongoing initialization (prevents double-init race condition)
let initPromise: Promise<void> | null = null;

/** Serializes writes for each setting so an older backend write cannot land last. */
const settingSaveLocks = new Map<keyof Settings, Promise<void>>();

/**
 * Initialize settings from database
 * Uses a promise lock to prevent race conditions if called multiple times
 */
export async function initSettings(): Promise<void> {
  if (isLoaded) return;

  // If already initializing, wait for that to complete
  if (initPromise) {
    return initPromise;
  }

  // Create the promise synchronously before yielding to the event loop,
  // preventing double-initialization races.
  const promise = (async () => {
    try {
      const stored = await loadSettings();
      settings = { ...DEFAULT_SETTINGS, ...stored };
      persistedSettings = { ...settings };
      isLoaded = true;
    } catch (error) {
      void logError("Failed to load settings", error);
      settings = { ...DEFAULT_SETTINGS };
      persistedSettings = { ...DEFAULT_SETTINGS };
      isLoaded = true;
    }
  })();

  initPromise = promise;

  try {
    await promise;
  } finally {
    initPromise = null;
  }
}

/**
 * Get current settings (reactive)
 */
export function getSettings(): Settings {
  return settings;
}

/** Persist first-run choices together; never unlock the app optimistically. */
export async function completeOnboarding(
  subjectIds: string[],
  showOptionalResults: boolean,
): Promise<boolean> {
  const optionalSubjectIds = [...new Set(subjectIds)];
  const validIds = new Set(
    MAINS_PAPER_TYPES.filter((paper) => paper.optional).map(
      (paper) => paper.id,
    ),
  );
  if (optionalSubjectIds.some((id) => !validIds.has(id))) return false;

  const patch = {
    optionalSubjectIds,
    showOptionalResults,
    hasCompletedOnboarding: true,
  };
  const keys = [
    "optionalSubjectIds",
    "showOptionalResults",
    "hasCompletedOnboarding",
  ] as const;
  const pending = keys.map((key) => settingSaveLocks.get(key));
  const operation = (async () => {
    await Promise.all(pending.map((save) => save?.catch(() => {})));
    // The native save_settings command writes this patch in one transaction.
    await saveSettings(patch);
    function commit<K extends keyof Settings>(key: K, value: Settings[K]) {
      persistedSettings[key] = value;
      if (settingSaveLocks.get(key) === operation) settings[key] = value;
    }
    keys.forEach((key) => commit(key, patch[key]));
  })();
  keys.forEach((key) => settingSaveLocks.set(key, operation));
  try {
    await operation;
    return true;
  } catch (error) {
    void logError("Failed to complete onboarding", error);
    return false;
  } finally {
    for (const key of keys) {
      if (settingSaveLocks.get(key) === operation) settingSaveLocks.delete(key);
    }
  }
}

/**
 * Update a single setting
 * Includes conditional rollback on failure - only rolls back if this is still
 * the latest write for the setting (prevents clobbering newer updates)
 */
export async function updateSetting<K extends keyof Settings>(
  key: K,
  value: Settings[K],
): Promise<boolean> {
  settings[key] = value;
  const previousSave = settingSaveLocks.get(key);
  const operation = (async () => {
    if (previousSave) await previousSave.catch(() => {});
    await saveSettings({ [key]: value });
    persistedSettings[key] = value;
  })();
  settingSaveLocks.set(key, operation);

  let saved = false;
  try {
    await operation;
    saved = true;
  } catch (error) {
    // A Svelte array/object is proxied, so reference equality with the input
    // cannot identify its write. The per-key operation tracks the latest edit.
    if (settingSaveLocks.get(key) === operation) {
      settings[key] = persistedSettings[key];
    }
    void logError(`Failed to save setting ${key}`, error);
  } finally {
    if (settingSaveLocks.get(key) === operation) {
      settingSaveLocks.delete(key);
    }
  }

  return saved;
}

// Export reactive getters for individual settings
export function getTheme(): Settings["theme"] {
  return settings.theme;
}
