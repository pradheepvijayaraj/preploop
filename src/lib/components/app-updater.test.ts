import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import AppUpdater from "./app-updater.svelte";

vi.stubGlobal(
  "ResizeObserver",
  class {
    observe() {}
    disconnect() {}
  },
);

const mocks = vi.hoisted(() => ({
  check: vi.fn(),
  getVersion: vi.fn(),
  logError: vi.fn(),
  listen: vi.fn(),
  invoke: vi.fn(),
  relaunch: vi.fn(),
  openUrl: vi.fn(),
  downloadPending: vi.fn(),
  installPending: vi.fn(),
  download: vi.fn(),
  install: vi.fn(),
  close: vi.fn(),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));
vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {
    onmessage?: (event: unknown) => void;
  },
  isTauri: () => true,
  invoke: mocks.invoke,
}));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: mocks.getVersion }));
vi.mock("$lib/services/logger", () => ({ logError: mocks.logError }));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: mocks.check }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: mocks.relaunch }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: mocks.openUrl }));

async function open() {
  render(AppUpdater);
  await screen.findByRole("dialog");
}
beforeEach(() => {
  vi.resetAllMocks();
  mocks.getVersion.mockResolvedValue("1.0.0");
  mocks.listen.mockResolvedValue(vi.fn());
  mocks.invoke.mockImplementation((command, args) => {
    if (command === "supports_in_app_updates") return Promise.resolve(true);
    if (command === "get_pending_update") return Promise.resolve(null);
    if (command === "download_pending_update")
      return mocks.downloadPending(args);
    if (command === "install_pending_update") return mocks.installPending(args);
    return Promise.resolve(true);
  });
  mocks.close.mockResolvedValue(undefined);
  mocks.downloadPending.mockResolvedValue({
    currentVersion: "1.0.0",
    version: "1.1.0",
    body: "release notes",
    size: 100,
    sha256: "test",
  });
  mocks.installPending.mockResolvedValue(undefined);
  mocks.download.mockResolvedValue(undefined);
  mocks.install.mockResolvedValue(undefined);
  mocks.relaunch.mockResolvedValue(undefined);
  mocks.check.mockResolvedValue({
    version: "1.1.0",
    body: "release notes",
    download: mocks.download,
    install: mocks.install,
    close: mocks.close,
  });
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("app updater", () => {
  it("downloads in the corner and preserves a deferred install without downloading again", async () => {
    let complete!: () => void;
    mocks.downloadPending.mockImplementation(({ onEvent }) => {
      onEvent.onmessage?.({
        event: "Started",
        data: { contentLength: 100 },
      });
      onEvent.onmessage?.({
        event: "Progress",
        data: { chunkLength: 42 },
      });
      return new Promise<void>((resolve) => {
        complete = resolve;
      });
    });
    const view = render(AppUpdater);
    await fireEvent.click(
      await screen.findByRole("button", { name: "DOWNLOAD" }),
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.getByRole("progressbar").getAttribute("aria-valuenow")).toBe(
      "42",
    );
    expect(
      screen.queryByRole("button", { name: "Update available" }),
    ).toBeNull();
    complete();
    await screen.findByRole("button", { name: "INSTALL NOW" });
    await fireEvent.click(screen.getByRole("button", { name: "LATER" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(
      screen
        .getByRole("button", { name: "Install update" })
        .classList.contains("update-trigger--ready"),
    ).toBe(true);
    await view.rerender({ home: false });
    expect(screen.queryByRole("button", { name: "Install update" })).toBeNull();
    await view.rerender({ home: true });
    await fireEvent.click(
      screen.getByRole("button", { name: "Install update" }),
    );
    await screen.findByRole("button", { name: "INSTALL NOW" });
    expect(mocks.downloadPending).toHaveBeenCalledTimes(1);
    expect(mocks.installPending).not.toHaveBeenCalled();
  });

  it("restores a downloaded update after the app is reopened", async () => {
    const pending = {
      currentVersion: "1.0.0",
      version: "1.1.0",
      body: "release notes",
      size: 100,
      sha256: "test",
    };
    let hasPendingUpdate = false;
    mocks.invoke.mockImplementation((command, args) => {
      if (command === "supports_in_app_updates") return Promise.resolve(true);
      if (command === "get_pending_update")
        return Promise.resolve(hasPendingUpdate ? pending : null);
      if (command === "download_pending_update") {
        hasPendingUpdate = true;
        return Promise.resolve(pending);
      }
      if (command === "install_pending_update")
        return mocks.installPending(args);
      return Promise.resolve(true);
    });
    const firstView = render(AppUpdater);
    await fireEvent.click(
      await screen.findByRole("button", { name: "DOWNLOAD" }),
    );
    await screen.findByRole("button", { name: "INSTALL NOW" });
    await fireEvent.click(screen.getByRole("button", { name: "LATER" }));
    firstView.unmount();

    render(AppUpdater);
    await screen.findByRole("button", { name: "Install update" });
    expect(mocks.downloadPending).not.toHaveBeenCalled();
    await fireEvent.click(
      screen.getByRole("button", { name: "Install update" }),
    );
    await screen.findByRole("button", { name: "INSTALL NOW" });
  });

  it("keeps a failed check compact without an empty version comparison", async () => {
    mocks.check.mockRejectedValue(new Error("offline"));
    render(AppUpdater, { home: false });
    await waitFor(() => expect(mocks.listen).toHaveBeenCalled());
    mocks.listen.mock.calls[0][1]();
    await screen.findByText("SOMETHING WENT WRONG");
    expect(screen.queryByText("LATEST VERSION")).toBeNull();
    expect(screen.queryByText("INSTALLED")).toBeNull();
    expect(screen.queryByRole("button", { name: "LATER" })).toBeNull();
    expect(screen.getByRole("button", { name: "OPEN GITHUB" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "TRY AGAIN" })).toBeTruthy();
  });

  it("does not claim there is no release when its manifest cannot be read", async () => {
    mocks.check.mockRejectedValue(new Error("request failed with status 404"));
    render(AppUpdater, { home: false });
    await waitFor(() => expect(mocks.listen).toHaveBeenCalled());
    mocks.listen.mock.calls[0][1]();
    await screen.findByText("SOMETHING WENT WRONG");
    expect(mocks.logError).toHaveBeenCalledWith(
      expect.stringContaining("fetching release information"),
      expect.objectContaining({ message: "request failed with status 404" }),
    );
    await fireEvent.click(screen.getByRole("button", { name: "OPEN GITHUB" }));
    expect(mocks.openUrl).toHaveBeenCalledWith(
      "https://github.com/utilinlabs/preploop/releases/latest",
    );
  });

  it.each(["getVersion", "invoke", "check"] as const)(
    "ends a stalled %s call within the whole-check deadline and ignores its late result",
    async (stage) => {
      let finish!: (value: unknown) => void;
      mocks[stage].mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            finish = resolve;
          }),
      );
      render(AppUpdater, { home: false });
      await waitFor(() => expect(mocks.listen).toHaveBeenCalled());
      vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
      await act(() => mocks.listen.mock.calls[0][1]());
      await waitFor(() => expect(mocks[stage]).toHaveBeenCalledTimes(1));
      await act(() => vi.advanceTimersByTimeAsync(35_000));
      expect(screen.getByText("SOMETHING WENT WRONG")).toBeTruthy();
      expect(mocks.logError).toHaveBeenCalledWith(
        expect.stringContaining("Update check timed out"),
      );

      // Retrying must be usable even if the old native promise is still pending.
      mocks.check.mockResolvedValue(null);
      await fireEvent.click(screen.getByRole("button", { name: "TRY AGAIN" }));
      await act(() => vi.advanceTimersByTimeAsync(0));
      expect(
        screen.getByRole("heading", { name: "YOU’RE UP TO DATE" }),
      ).toBeTruthy();
      const checksAfterRetry = mocks.check.mock.calls.length;
      const lateClose = vi.fn().mockResolvedValue(undefined);
      await act(() =>
        finish(
          stage === "getVersion"
            ? "0.0.1"
            : stage === "invoke"
              ? false
              : {
                  version: "9.9.9",
                  close: lateClose,
                },
        ),
      );
      expect(
        screen.getByRole("heading", { name: "YOU’RE UP TO DATE" }),
      ).toBeTruthy();
      expect(screen.queryByText("9.9.9")).toBeNull();
      expect(screen.queryByText("0.0.1")).toBeNull();
      expect(mocks.check).toHaveBeenCalledTimes(checksAfterRetry);
      if (stage === "check") expect(lateClose).toHaveBeenCalledOnce();
    },
  );

  it("clears the deadline and closes late update resources after unmount", async () => {
    let finish!: (value: unknown) => void;
    mocks.check.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    const view = render(AppUpdater, { home: false });
    await waitFor(() => expect(mocks.listen).toHaveBeenCalled());
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    await act(() => mocks.listen.mock.calls[0][1]());
    await waitFor(() => expect(mocks.check).toHaveBeenCalledTimes(1));
    view.unmount();
    await act(() => vi.advanceTimersByTimeAsync(20_000));
    expect(mocks.logError).not.toHaveBeenCalled();
    await act(() => finish({ version: "1.1.0", close: mocks.close }));
    expect(mocks.close).toHaveBeenCalledOnce();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("does not block a new check on releasing an old update resource", async () => {
    await open();
    mocks.close.mockImplementationOnce(() => new Promise(() => {}));
    mocks.check.mockResolvedValue(null);
    mocks.listen.mock.calls[0][1]();
    await screen.findByRole("heading", { name: "YOU’RE UP TO DATE" });
    expect(mocks.check).toHaveBeenLastCalledWith({ timeout: 30_000 });
  });

  it("shows the menu result outside home without exposing the home button", async () => {
    mocks.check.mockResolvedValue(null);
    render(AppUpdater, { home: false });
    await waitFor(() => expect(mocks.listen).toHaveBeenCalled());
    mocks.listen.mock.calls[0][1]();
    await screen.findByRole("heading", { name: "YOU’RE UP TO DATE" });
    expect(screen.getAllByText("1.0.0")).toHaveLength(2);
    expect(screen.getAllByRole("button")).toHaveLength(1);
    expect(
      screen.queryByRole("button", { name: "Update available" }),
    ).toBeNull();
    await fireEvent.click(
      screen.getByRole("button", { name: "Close update dialog" }),
    );
    expect(screen.queryByRole("dialog")).toBeNull();
  });
  it("prevents a menu-triggered update from restarting an active session", async () => {
    const view = render(AppUpdater, { home: false });
    await waitFor(() => expect(mocks.listen).toHaveBeenCalled());
    mocks.listen.mock.calls[0][1]();
    await fireEvent.click(
      await screen.findByRole("button", { name: "DOWNLOAD" }),
    );
    const install = await screen.findByRole("button", {
      name: "INSTALL NOW",
    });
    expect((install as HTMLButtonElement).disabled).toBe(true);
    await fireEvent.click(install);
    expect(mocks.installPending).not.toHaveBeenCalled();
    await view.rerender({ home: true });
    expect((install as HTMLButtonElement).disabled).toBe(false);
  });

  it("requires explicit download and install actions before restarting", async () => {
    await open();
    await screen.findByText("1.1.0");
    expect(mocks.download).not.toHaveBeenCalled();
    await fireEvent.click(screen.getByRole("button", { name: "DOWNLOAD" }));
    const install = await screen.findByRole("button", {
      name: "INSTALL NOW",
    });
    expect(mocks.install).not.toHaveBeenCalled();
    await fireEvent.click(install);
    await waitFor(() => expect(mocks.relaunch).toHaveBeenCalledTimes(1));
    expect(mocks.installPending).toHaveBeenCalledTimes(1);
  });
  it("never installs after a download or signature failure", async () => {
    mocks.downloadPending.mockRejectedValue(new Error("bad signature"));
    await open();
    await fireEvent.click(
      await screen.findByRole("button", { name: "DOWNLOAD" }),
    );
    await screen.findByText("SOMETHING WENT WRONG");
    expect(mocks.installPending).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: "INSTALL NOW" })).toBeNull();
  });
  it("offers GitHub recovery for unsupported package formats", async () => {
    mocks.invoke.mockResolvedValue(false);
    await open();
    await screen.findByText("SOMETHING WENT WRONG");
    expect(mocks.check).toHaveBeenCalledTimes(1);
    await fireEvent.click(screen.getByRole("button", { name: "OPEN GITHUB" }));
    expect(mocks.openUrl).toHaveBeenCalledWith(
      "https://github.com/utilinlabs/preploop/releases/latest",
    );
  });
  it.each([null, new Error("offline")])(
    "stays invisible when no update is confirmed: %s",
    async (result) => {
      if (result instanceof Error) mocks.check.mockRejectedValue(result);
      else mocks.check.mockResolvedValue(result);
      render(AppUpdater);
      await waitFor(() => expect(mocks.check).toHaveBeenCalledTimes(1));
      expect(
        screen.queryByRole("button", { name: "Update available" }),
      ).toBeNull();
      expect(screen.queryByRole("dialog")).toBeNull();
    },
  );
  it("only checks and displays on home and keeps a dismissed update available", async () => {
    const view = render(AppUpdater, { home: false });
    expect(mocks.check).not.toHaveBeenCalled();
    await view.rerender({ home: true });
    await screen.findByRole("dialog");
    await fireEvent.click(screen.getByRole("button", { name: "LATER" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(
      screen.getByRole("button", { name: "Update available" }),
    ).toBeTruthy();
    await view.rerender({ home: false });
    expect(
      screen.queryByRole("button", { name: "Update available" }),
    ).toBeNull();
    await view.rerender({ home: true });
    expect(mocks.check).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("dialog")).toBeNull();
  });
  it("shows a generic error when the automatic relaunch fails", async () => {
    mocks.relaunch.mockRejectedValue(new Error("restart failed"));
    await open();
    await fireEvent.click(
      await screen.findByRole("button", { name: "DOWNLOAD" }),
    );
    await fireEvent.click(
      await screen.findByRole("button", { name: "INSTALL NOW" }),
    );
    await screen.findByText("SOMETHING WENT WRONG");
    await waitFor(() => expect(mocks.relaunch).toHaveBeenCalledTimes(1));
    expect(mocks.installPending).toHaveBeenCalledTimes(1);
  });
});
