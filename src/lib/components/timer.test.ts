// @vitest-environment jsdom

import { fireEvent, render, screen } from "@testing-library/svelte";
import { describe, expect, it, vi } from "vitest";
import Timer from "$lib/components/timer.svelte";

describe("Timer", () => {
  it("exposes the icon-only pause action by name", async () => {
    const onPause = vi.fn();
    render(Timer, {
      props: {
        timeRemaining: 300,
        isPaused: false,
        onPause,
      },
    });

    const pauseButton = screen.getByRole("button", { name: "Pause test" });
    await fireEvent.click(pauseButton);

    expect(onPause).toHaveBeenCalledOnce();
  });
});
