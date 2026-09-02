// @vitest-environment jsdom

import { fireEvent, render, screen } from "@testing-library/svelte";
import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  settings: {
    theme: "system" as const,
    navigatorExpanded: false,
    lastLibrarySelectionId: null,
    practiceShowImmediateFeedback: true,
    autoSubmitOnTimerEnd: true,
    optionalSubjectIds: [] as string[],
    showOptionalResults: false,
    hasCompletedOnboarding: true,
  },
  updateSetting: vi.fn(),
  openUrl: vi.fn(),
}));

vi.mock("$lib/stores/settings.svelte", () => ({
  getSettings: () => mocks.settings,
  updateSetting: mocks.updateSetting,
}));

vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: mocks.openUrl,
}));

import OptionalPreferencesModal from "$lib/components/optional-preferences-modal.svelte";

describe("OptionalPreferencesModal", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.updateSetting.mockResolvedValue(true);
    mocks.settings.optionalSubjectIds = [];
    mocks.settings.hasCompletedOnboarding = true;
  });

  it("opens the settings dialog from the footer control", async () => {
    render(OptionalPreferencesModal);

    const trigger = screen.getByRole("button", { name: "Study preferences" });
    expect(trigger.getAttribute("aria-expanded")).toBe("false");

    await fireEvent.click(trigger);

    expect(trigger.getAttribute("aria-expanded")).toBe("true");

    expect(
      await screen.findByRole("heading", { name: "SETTINGS" }),
    ).toBeTruthy();
  });

  it("does not launch onboarding as a Settings side effect", () => {
    mocks.settings.hasCompletedOnboarding = false;
    render(OptionalPreferencesModal);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("preserves the ability to remove the final optional in Settings", async () => {
    mocks.settings.optionalSubjectIds = ["geography"];
    render(OptionalPreferencesModal);
    await fireEvent.click(
      screen.getByRole("button", { name: "Study preferences" }),
    );
    await fireEvent.click(screen.getByRole("button", { name: "Geography" }));
    expect(mocks.updateSetting).toHaveBeenCalledWith("optionalSubjectIds", []);
  });
});
