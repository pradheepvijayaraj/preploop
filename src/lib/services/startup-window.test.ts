import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { loadStartupTheme, revealStartupWindow } from "./startup-window";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  isTauri: vi.fn(),
  logError: vi.fn(),
  initSettings: vi.fn(),
  getTheme: vi.fn(),
  setMode: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
  isTauri: mocks.isTauri,
}));
vi.mock("mode-watcher", () => ({ setMode: mocks.setMode }));
vi.mock("$lib/services/logger", () => ({ logError: mocks.logError }));
vi.mock("$lib/stores/settings.svelte", () => ({
  initSettings: mocks.initSettings,
  getTheme: mocks.getTheme,
}));

let frames: FrameRequestCallback[];
beforeEach(() => {
  vi.resetAllMocks();
  vi.useFakeTimers();
  frames = [];
  vi.stubGlobal(
    "requestAnimationFrame",
    vi.fn((callback) => frames.push(callback)),
  );
  vi.stubGlobal("cancelAnimationFrame", vi.fn());
  mocks.isTauri.mockReturnValue(true);
  mocks.invoke.mockResolvedValue(undefined);
  mocks.initSettings.mockResolvedValue(undefined);
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("startup window", () => {
  it.each(["dark", "light", "system"])(
    "loads the saved %s theme before applying it",
    async (theme) => {
      let finish!: () => void;
      mocks.initSettings.mockImplementation(
        () =>
          new Promise<void>((resolve) => {
            finish = resolve;
          }),
      );
      mocks.getTheme.mockReturnValue(theme);
      const loading = loadStartupTheme();
      expect(mocks.setMode).not.toHaveBeenCalled();
      finish();
      await loading;
      expect(mocks.setMode).toHaveBeenCalledWith(theme);
    },
  );

  it("waits for the themed DOM and two frames before revealing the window", async () => {
    const ready = revealStartupWindow();
    await vi.advanceTimersByTimeAsync(0);
    expect(mocks.invoke).not.toHaveBeenCalled();
    frames.shift()!(0);
    expect(mocks.invoke).not.toHaveBeenCalled();
    frames.shift()!(16);
    await ready;
    expect(mocks.invoke).toHaveBeenCalledExactlyOnceWith("startup_ready");
    await vi.advanceTimersByTimeAsync(500);
    expect(mocks.invoke).toHaveBeenCalledOnce();
  });

  it("still reveals when hidden WebViews suspend animation frames", async () => {
    const ready = revealStartupWindow();
    await vi.advanceTimersByTimeAsync(100);
    await ready;
    expect(mocks.invoke).toHaveBeenCalledExactlyOnceWith("startup_ready");
    expect(cancelAnimationFrame).toHaveBeenCalled();
  });

  it("does not send native window commands in browser previews", async () => {
    mocks.isTauri.mockReturnValue(false);
    await revealStartupWindow();
    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(requestAnimationFrame).not.toHaveBeenCalled();
  });

  it("logs reveal failures without throwing an unhandled rejection", async () => {
    mocks.invoke.mockRejectedValue(new Error("IPC unavailable"));
    const ready = revealStartupWindow();
    await vi.advanceTimersByTimeAsync(100);
    await expect(ready).resolves.toBeUndefined();
    expect(mocks.logError).toHaveBeenCalledWith(
      "Could not reveal the startup window",
      expect.any(Error),
    );
  });
});
