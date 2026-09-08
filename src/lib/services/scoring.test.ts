import { beforeEach, describe, expect, it, vi } from "vitest";

const { toggleFlagMock } = vi.hoisted(() => ({ toggleFlagMock: vi.fn() }));

vi.mock("$lib/services/test-session", () => ({
  toggleFlag: toggleFlagMock,
}));

import type { QuestionReviewItem } from "$lib/types";
import { filterReviewItems, toggleReviewItemFlag } from "$lib/services/scoring";

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

describe("review result classification", () => {
  const reviewItem = (
    id: string,
    isCorrect: boolean,
    userAnswer: string | null,
  ) =>
    ({
      question: { id },
      userAnswer,
      isCorrect,
      isFlagged: false,
      marksObtained: isCorrect ? 2 : 0,
    }) as QuestionReviewItem;

  it("does not classify an unanswered withdrawn credit as skipped", () => {
    const items = [
      reviewItem("withdrawn", true, null),
      reviewItem("skipped", false, null),
      reviewItem("answered", false, "a"),
    ];

    expect(
      filterReviewItems(items, "correct").map((item) => item.question.id),
    ).toEqual(["withdrawn"]);
    expect(
      filterReviewItems(items, "unanswered").map((item) => item.question.id),
    ).toEqual(["skipped"]);
    expect(
      filterReviewItems(items, "wrong").map((item) => item.question.id),
    ).toEqual(["answered"]);
  });
});
