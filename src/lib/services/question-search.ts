import { invokeBackend } from "$lib/services/backend";
import type { QuestionSearchResponse } from "$lib/types";

export async function searchQuestions(
  query: string,
  sections?: string[],
): Promise<QuestionSearchResponse> {
  return invokeBackend<QuestionSearchResponse>("search_questions", {
    query,
    sections,
  });
}

/** Build the reusable search service and run one real embedding inference. */
export async function warmQuestionSearch(): Promise<void> {
  await invokeBackend<void>("warm_question_search");
}
