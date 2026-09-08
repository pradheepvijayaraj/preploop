/**
 * Scoring service \u2014 frontend wrappers for result calculation and review.
 *
 * The actual scoring logic lives in the Rust backend (scoring.rs).
 * These functions simply invoke the backend commands and return the results.
 */
import type { QuestionReviewItem, TestResult } from "$lib/types";
import { invokeBackend } from "$lib/services/backend";
import { toggleFlag } from "$lib/services/test-session";

export interface ReviewFlagUpdate {
  questionId: string;
  isFlagged: boolean;
}

export async function toggleReviewItemFlag(
  attemptId: string,
  questionId: string,
): Promise<ReviewFlagUpdate> {
  return {
    questionId,
    isFlagged: await toggleFlag(attemptId, questionId),
  };
}

export async function calculateTestResult(
  attemptId: string,
): Promise<TestResult> {
  return invokeBackend<TestResult>("calculate_test_result", { attemptId });
}

export async function getQuestionReview(
  attemptId: string,
): Promise<QuestionReviewItem[]> {
  return invokeBackend<QuestionReviewItem[]>("get_question_review", {
    attemptId,
  });
}

export function filterReviewItems(
  items: QuestionReviewItem[],
  filter: "all" | "correct" | "wrong" | "flagged" | "unanswered",
): QuestionReviewItem[] {
  switch (filter) {
    case "correct":
      return items.filter((item) => item.isCorrect);
    case "wrong":
      return items.filter(
        (item) => !item.isCorrect && item.userAnswer !== null,
      );
    case "flagged":
      return items.filter((item) => item.isFlagged);
    case "unanswered":
      return items.filter((item) => item.userAnswer === null);
    default:
      return items;
  }
}
