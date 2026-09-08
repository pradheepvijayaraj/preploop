import { beforeEach, describe, expect, it, vi } from "vitest";

const { toggleFlagMock } = vi.hoisted(() => ({ toggleFlagMock: vi.fn() }));

vi.mock("$lib/services/test-session", () => ({
  toggleFlag: toggleFlagMock,
}));

import { toggleReviewItemFlag } from "$lib/services/scoring";

describe("review flag updates", () => {
  beforeEach(() => {
    toggleFlagMock.mockReset();
  });

  it("keeps the requested question identity across an asynchronous toggle", async () => {
    let resolveToggle: ((flagged: boolean) => void) | undefined;
    toggleFlagMock.mockReturnValue(
      new Promise<boolean>((resolve) => {
        resolveToggle = resolve;
      }),
    );

    const update = toggleReviewItemFlag("attempt-1", "question-1");

    resolveToggle?.(true);

    await expect(update).resolves.toEqual({
      questionId: "question-1",
      isFlagged: true,
    });
    expect(toggleFlagMock).toHaveBeenCalledWith("attempt-1", "question-1");
  });
});
