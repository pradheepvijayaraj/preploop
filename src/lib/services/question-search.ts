import { invokeBackend } from "$lib/services/backend";
import type { QuestionSearchResponse } from "$lib/types";
import { Channel } from "@tauri-apps/api/core";

export interface SearchControls {
  clientId: string;
  requestId: number;
  onProgress: (response: QuestionSearchResponse) => void;
}

export async function searchQuestions(
  query: string,
  sections?: string[],
  controls?: SearchControls,
): Promise<QuestionSearchResponse> {
  const onProgress = new Channel<QuestionSearchResponse>();
  let settled = false;
  onProgress.onmessage = (response) => {
    if (!settled) controls?.onProgress(response);
  };
  try {
    return await invokeBackend<QuestionSearchResponse>(
      "search_questions",
      {
        query,
        sections,
        ...(controls
          ? { clientId: controls.clientId, requestId: controls.requestId }
          : {}),
      },
      { onProgress },
    );
  } finally {
    settled = true;
  }
}

export async function cancelQuestionSearch(
  clientId: string,
  requestId: number,
): Promise<void> {
  await invokeBackend<void>("cancel_question_search", { clientId, requestId });
}

/** Build the reusable search service and run one real embedding inference. */
export async function warmQuestionSearch(): Promise<void> {
  await invokeBackend<void>("warm_question_search");
}
