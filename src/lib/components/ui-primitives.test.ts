// @vitest-environment jsdom

import { fireEvent, render, screen } from "@testing-library/svelte";
import { describe, expect, it } from "vitest";
import UiPrimitivesHarness from "$lib/components/test-helpers/ui-primitives-harness.svelte";

describe("UI primitive wrappers", () => {
  it("keeps labels, roles, and checked state synchronized", async () => {
    render(UiPrimitivesHarness);

    const toggle = screen.getByRole("switch", {
      name: "Immediate feedback",
    });
    const checkbox = screen.getByRole("checkbox", { name: "Select answer" });

    expect(toggle.getAttribute("aria-checked")).toBe("false");
    expect(checkbox.getAttribute("aria-checked")).toBe("false");

    await fireEvent.click(toggle);
    await fireEvent.click(checkbox);

    expect(toggle.getAttribute("aria-checked")).toBe("true");
    expect(checkbox.getAttribute("aria-checked")).toBe("true");
    const state = screen.getByTestId("primitive-state").textContent;
    expect(state).toContain("switch-on");
    expect(state).toContain("checkbox-on");
  });
});
