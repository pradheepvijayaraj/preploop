import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import Onboarding from "$lib/components/test-helpers/onboarding-harness.svelte";
import { getSettings, updateSetting } from "$lib/stores/settings.svelte";

const mocks = vi.hoisted(() => ({ saveSettings: vi.fn(), logError: vi.fn() }));
vi.mock("$lib/services/settings-persistence", () => ({
  loadSettings: vi.fn(),
  saveSettings: mocks.saveSettings,
}));
vi.mock("$lib/services/logger", () => ({ logError: mocks.logError }));

beforeEach(async () => {
  vi.resetAllMocks();
  mocks.saveSettings.mockResolvedValue(undefined);
  await updateSetting("hasCompletedOnboarding", false);
  await updateSetting("optionalSubjectIds", []);
  await updateSetting("showOptionalResults", false);
  mocks.saveSettings.mockClear();
});
afterEach(cleanup);

describe("embedded first-run onboarding", () => {
  it("uses the original short subject labels with only a brief Settings note", () => {
    render(Onboarding);
    for (const [name, label] of [
      ["Anthropology", "Anthro"],
      ["Commerce & Accountancy", "Commerce"],
      ["Mathematics", "Maths"],
      ["Medical Science", "Medical"],
      ["Political Science & International Relations", "PSIR"],
      ["Public Administration", "Pub Ad"],
    ]) {
      const button = screen.getByRole("button", { name });
      expect(button.textContent?.trim()).toBe(label);
      expect(button.querySelector("svg")).toBeNull();
    }
    const note = screen.getByText("CHANGE THESE ANYTIME IN SETTINGS");
    expect(note.closest("header")).toBeNull();
    expect(note.closest("fieldset")).toBeNull();
    expect(screen.getByRole("heading", { name: "UPSC CSE" })).toBeTruthy();
    expect(screen.getByText("CHOOSE OPTIONAL")).toBeTruthy();
    for (const feature of [
      "GROWING PYQ LIBRARY",
      "CONTEXTUAL SEARCH",
      "FOCUSED PRACTICE",
      "TIMED TESTS",
      "LOCAL FIRST BY DESIGN",
      "NO FUSS",
      "MORE ON THE WAY...",
    ]) {
      expect(screen.getByText(feature)).toBeTruthy();
    }
    const repositoryLink = screen.getByRole("link", {
      name: "Open PrepLoop on GitHub",
    });
    expect(repositoryLink.textContent).toContain("OPEN SOURCE");
    expect(repositoryLink.getAttribute("href")).toBe(
      "https://github.com/utilinlabs/preploop",
    );
    expect(repositoryLink.querySelector("svg")).toBeTruthy();
    expect(
      screen
        .getByRole("heading", { name: "MAKE PREPLOOP YOURS" })
        .querySelector("svg"),
    ).toBeTruthy();
    expect(note.classList.contains("font-medium")).toBe(true);
    expect(
      screen
        .getByRole("region", { name: "MAKE PREPLOOP YOURS" })
        .querySelector('hr, [role="separator"], .border-t, .border-b'),
    ).toBeNull();
    expect(screen.queryByText("GET STARTED")).toBeNull();
    const continueButton = screen.getByRole("button", { name: "Continue" });
    expect(continueButton.querySelector("svg")).toBeNull();
    expect(continueButton.className).not.toContain("w-full");
  });

  it("renders before the main app and cannot be dismissed by outside clicks or Escape", async () => {
    render(Onboarding);
    expect(screen.queryByTestId("main-app")).toBeNull();
    expect(
      screen.getByRole("heading", { name: "MAKE PREPLOOP YOURS" }),
    ).toBeTruthy();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.queryByRole("button", { name: /close|skip/i })).toBeNull();
    await fireEvent.click(document.body);
    await fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByTestId("main-app")).toBeNull();
    expect(
      screen.getByRole("heading", { name: "MAKE PREPLOOP YOURS" }),
    ).toBeTruthy();
  });

  it("allows continuing without selecting any optional subjects", async () => {
    render(Onboarding);
    const next = screen.getByRole("button", {
      name: "Continue",
    }) as HTMLButtonElement;
    expect(next.disabled).toBe(false);
    await fireEvent.click(next);
    await screen.findByTestId("main-app");
    expect(mocks.saveSettings).toHaveBeenCalledOnce();
    expect(mocks.saveSettings).toHaveBeenCalledWith({
      optionalSubjectIds: [],
      showOptionalResults: false,
      hasCompletedOnboarding: true,
    });
  });

  it("allows clearing all selected subjects before continuing", async () => {
    render(Onboarding);
    const next = screen.getByRole("button", {
      name: "Continue",
    }) as HTMLButtonElement;
    await fireEvent.click(screen.getByRole("button", { name: "Geography" }));
    expect(next.disabled).toBe(false);
    await fireEvent.click(screen.getByRole("button", { name: "History" }));
    await fireEvent.click(screen.getByRole("button", { name: "Geography" }));
    expect(next.disabled).toBe(false);
    await fireEvent.click(screen.getByRole("button", { name: "History" }));
    expect(next.disabled).toBe(false);
    await fireEvent.click(next);
    await screen.findByTestId("main-app");
    expect(getSettings().optionalSubjectIds).toEqual([]);
  });

  it("keeps the main app unmounted until all preferences are saved successfully", async () => {
    let finish!: () => void;
    mocks.saveSettings.mockImplementationOnce(
      () =>
        new Promise<void>((resolve) => {
          finish = resolve;
        }),
    );
    render(Onboarding);
    await fireEvent.click(screen.getByRole("button", { name: "Geography" }));
    await fireEvent.click(screen.getByRole("button", { name: "History" }));
    await fireEvent.click(screen.getByRole("switch"));
    await fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    await waitFor(() => expect(mocks.saveSettings).toHaveBeenCalledOnce());
    expect(mocks.saveSettings).toHaveBeenCalledWith({
      optionalSubjectIds: ["geography", "history"],
      showOptionalResults: true,
      hasCompletedOnboarding: true,
    });
    expect(getSettings().hasCompletedOnboarding).toBe(false);
    expect(screen.queryByTestId("main-app")).toBeNull();
    expect(
      (screen.getByRole("button", { name: "Saving…" }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
    finish();
    await screen.findByTestId("main-app");
    expect(
      screen.queryByRole("heading", { name: "MAKE PREPLOOP YOURS" }),
    ).toBeNull();
    cleanup();
    render(Onboarding);
    expect(screen.getByTestId("main-app")).toBeTruthy();
    expect(
      screen.queryByRole("heading", { name: "MAKE PREPLOOP YOURS" }),
    ).toBeNull();
  });

  it("keeps choices and onboarding visible after a save failure, allowing retry", async () => {
    mocks.saveSettings.mockRejectedValueOnce(new Error("disk full"));
    render(Onboarding);
    await fireEvent.click(screen.getByRole("button", { name: "Geography" }));
    await fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    await screen.findByRole("alert");
    expect(screen.queryByTestId("main-app")).toBeNull();
    expect(getSettings().hasCompletedOnboarding).toBe(false);
    expect(getSettings().optionalSubjectIds).toEqual([]);
    expect(
      screen
        .getByRole("button", { name: "Geography" })
        .getAttribute("aria-pressed"),
    ).toBe("true");
    await fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    await screen.findByTestId("main-app");
  });

  it("does not force existing users back through setup after changing Settings", async () => {
    await updateSetting("hasCompletedOnboarding", true);
    render(Onboarding);
    expect(screen.getByTestId("main-app")).toBeTruthy();
    expect(
      screen.queryByRole("heading", { name: "MAKE PREPLOOP YOURS" }),
    ).toBeNull();
  });
});
