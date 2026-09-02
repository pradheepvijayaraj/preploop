import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  loadSettings: vi.fn(),
  saveSettings: vi.fn(),
  invoke: vi.fn(),
  logError: vi.fn(),
}));
vi.mock("$lib/services/settings-persistence", () => ({
  loadSettings: mocks.loadSettings,
  saveSettings: mocks.saveSettings,
}));
vi.mock("@tauri-apps/api/core", () => ({
  isTauri: () => true,
  invoke: mocks.invoke,
}));
vi.mock("$lib/services/logger", () => ({ logError: mocks.logError }));

let dispose: (() => Promise<void>) | undefined;
beforeEach(() => {
  vi.resetModules();
  vi.resetAllMocks();
  const cached = new Map<string, string>();
  // Keep the browser cache deterministic and independent of Node's Web Storage.
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => cached.get(key) ?? null,
    setItem: (key: string, value: string) => cached.set(key, value),
    removeItem: (key: string) => cached.delete(key),
    clear: () => cached.clear(),
    key: (index: number) => [...cached.keys()][index] ?? null,
    get length() {
      return cached.size;
    },
  } satisfies Storage);
  mocks.loadSettings.mockResolvedValue({});
});
afterEach(async () => {
  await dispose?.();
  dispose = undefined;
  vi.unstubAllGlobals();
  document.documentElement.classList.remove("dark");
  document.documentElement.style.removeProperty("color-scheme");
});

describe("first-run system theme", () => {
  it.each(["light", "dark"] as const)(
    "uses the system's %s appearance before reveal, even with an old cached override",
    async (systemMode) => {
      vi.stubGlobal(
        "matchMedia",
        vi.fn((query: string) => ({
          matches:
            query.includes("prefers-color-scheme: light") &&
            systemMode === "light",
          media: query,
          onchange: null,
          addListener: vi.fn(),
          removeListener: vi.fn(),
          addEventListener: vi.fn(),
          removeEventListener: vi.fn(),
          dispatchEvent: () => true,
        })),
      );
      // This is the test's isolated DOM storage, not an installed app profile.
      localStorage.setItem(
        "mode-watcher-mode",
        systemMode === "light" ? "dark" : "light",
      );
      const { ModeWatcher, userPrefersMode } = await import("mode-watcher");
      // Import the renderer after resetModules too, so it shares the component's runtime.
      const { mount, tick, unmount } = await import("svelte");
      const { loadStartupTheme, revealStartupWindow } =
        await import("./startup-window");
      const { getSettings } = await import("$lib/stores/settings.svelte");
      const target = document.createElement("div");
      document.body.append(target);
      const instance = mount(ModeWatcher, {
        target,
        props: { defaultMode: "system", synchronousModeChanges: true },
      });
      dispose = async () => {
        await unmount(instance);
        target.remove();
      };
      await tick();

      mocks.invoke.mockImplementation(async (command: string) => {
        expect(command).toBe("startup_ready");
        expect(userPrefersMode.current).toBe("system");
        expect(document.documentElement.style.colorScheme).toBe(systemMode);
        expect(document.documentElement.classList.contains("dark")).toBe(
          systemMode === "dark",
        );
      });
      await loadStartupTheme();
      expect(getSettings().theme).toBe("system");
      expect(getSettings().hasCompletedOnboarding).toBe(false);
      await revealStartupWindow();
      expect(mocks.invoke).toHaveBeenCalledExactlyOnceWith("startup_ready");
      expect(mocks.logError).not.toHaveBeenCalled();
      expect(mocks.saveSettings).not.toHaveBeenCalled();
    },
  );
});
