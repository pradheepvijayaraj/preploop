import { beforeEach, describe, expect, it, vi } from "vitest";

const persistenceMocks = vi.hoisted(() => ({
  loadSettings: vi.fn(),
  saveSettings: vi.fn(),
  logError: vi.fn(),
}));

vi.mock("$lib/services/settings-persistence", () => ({
  loadSettings: persistenceMocks.loadSettings,
  saveSettings: persistenceMocks.saveSettings,
}));
vi.mock("$lib/services/logger", () => ({
  logError: persistenceMocks.logError,
}));

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

describe("settings store", () => {
  beforeEach(() => {
    vi.resetModules();
    vi.clearAllMocks();
    persistenceMocks.saveSettings.mockResolvedValue(undefined);
  });

  it("shares one initialization and merges stored settings with defaults", async () => {
    const load = deferred<{ theme: "dark" }>();
    persistenceMocks.loadSettings.mockImplementationOnce(() => load.promise);
    const store = await import("$lib/stores/settings.svelte");

    const firstInit = store.initSettings();
    const secondInit = store.initSettings();
    expect(persistenceMocks.loadSettings).toHaveBeenCalledTimes(1);

    load.resolve({ theme: "dark" });
    await Promise.all([firstInit, secondInit]);

    expect(store.getSettings()).toMatchObject({
      theme: "dark",
      navigatorExpanded: false,
      practiceShowImmediateFeedback: true,
      autoSubmitOnTimerEnd: true,
      optionalSubjectIds: [],
      showOptionalResults: false,
      hasCompletedOnboarding: false,
    });
  });

  it("falls back to defaults when loading fails", async () => {
    persistenceMocks.loadSettings.mockRejectedValueOnce(new Error("offline"));
    const store = await import("$lib/stores/settings.svelte");

    await store.initSettings();

    expect(store.getSettings().theme).toBe("system");
    expect(persistenceMocks.logError).toHaveBeenCalledOnce();
  });

  it("rolls back a failed optimistic update", async () => {
    persistenceMocks.saveSettings.mockRejectedValueOnce(
      new Error("write failed"),
    );
    const store = await import("$lib/stores/settings.svelte");

    const saved = await store.updateSetting("theme", "dark");

    expect(saved).toBe(false);
    expect(store.getSettings().theme).toBe("system");
  });

  it("does not let an older failed write clobber a newer value", async () => {
    const firstWrite = deferred<void>();
    persistenceMocks.saveSettings
      .mockImplementationOnce(() => firstWrite.promise)
      .mockResolvedValueOnce(undefined);
    const store = await import("$lib/stores/settings.svelte");

    const firstUpdate = store.updateSetting("theme", "dark");
    const secondUpdate = store.updateSetting("theme", "light");
    expect(persistenceMocks.saveSettings).toHaveBeenCalledTimes(1);
    firstWrite.reject(new Error("old write failed"));
    await Promise.all([firstUpdate, secondUpdate]);

    expect(persistenceMocks.saveSettings).toHaveBeenCalledTimes(2);
    expect(store.getSettings().theme).toBe("light");
  });

  it("rolls back to the last persisted value when queued writes both fail", async () => {
    const firstWrite = deferred<void>();
    persistenceMocks.saveSettings
      .mockImplementationOnce(() => firstWrite.promise)
      .mockRejectedValueOnce(new Error("new write failed"));
    const store = await import("$lib/stores/settings.svelte");

    const firstUpdate = store.updateSetting("theme", "dark");
    const secondUpdate = store.updateSetting("theme", "light");
    firstWrite.reject(new Error("old write failed"));
    await Promise.all([firstUpdate, secondUpdate]);

    expect(store.getSettings().theme).toBe("system");
  });

  it.each([{ ids: ["unknown"] }, { ids: ["essay"] }])(
    "rejects invalid onboarding optionals: %j",
    async ({ ids }) => {
      const store = await import("$lib/stores/settings.svelte");
      expect(await store.completeOnboarding(ids, false)).toBe(false);
      expect(persistenceMocks.saveSettings).not.toHaveBeenCalled();
      expect(store.getSettings().hasCompletedOnboarding).toBe(false);
    },
  );

  it("completes onboarding with no optional subjects selected", async () => {
    const store = await import("$lib/stores/settings.svelte");
    expect(await store.completeOnboarding([], false)).toBe(true);
    expect(persistenceMocks.saveSettings).toHaveBeenCalledWith({
      optionalSubjectIds: [],
      showOptionalResults: false,
      hasCompletedOnboarding: true,
    });
    expect(store.getSettings().hasCompletedOnboarding).toBe(true);
    expect(store.getSettings().optionalSubjectIds).toEqual([]);
  });

  it("saves onboarding atomically without exposing completion while pending", async () => {
    const write = deferred<void>();
    persistenceMocks.saveSettings.mockImplementationOnce(() => write.promise);
    const store = await import("$lib/stores/settings.svelte");
    const saving = store.completeOnboarding(["geography", "geography"], true);
    await vi.waitFor(() =>
      expect(persistenceMocks.saveSettings).toHaveBeenCalledOnce(),
    );
    expect(persistenceMocks.saveSettings).toHaveBeenCalledWith({
      optionalSubjectIds: ["geography"],
      showOptionalResults: true,
      hasCompletedOnboarding: true,
    });
    expect(store.getSettings().hasCompletedOnboarding).toBe(false);
    write.resolve();
    expect(await saving).toBe(true);
    expect(store.getSettings().hasCompletedOnboarding).toBe(true);
  });

  it("keeps per-setting writes ordered after onboarding and rolls back to its saved values", async () => {
    const write = deferred<void>();
    persistenceMocks.saveSettings
      .mockImplementationOnce(() => write.promise)
      .mockRejectedValueOnce(new Error("later write failed"));
    const store = await import("$lib/stores/settings.svelte");
    const onboarding = store.completeOnboarding(["geography"], true);
    const setting = store.updateSetting("optionalSubjectIds", ["history"]);
    await vi.waitFor(() =>
      expect(persistenceMocks.saveSettings).toHaveBeenCalledOnce(),
    );
    expect(store.getSettings().optionalSubjectIds).toEqual(["history"]);
    write.resolve();
    await Promise.all([onboarding, setting]);
    expect(persistenceMocks.saveSettings).toHaveBeenCalledTimes(2);
    expect(store.getSettings().optionalSubjectIds).toEqual(["geography"]);
    expect(store.getSettings().hasCompletedOnboarding).toBe(true);
  });
});
