import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
  Channel: class {
    onmessage?: (value: unknown) => void;
  },
}));

import {
  searchQuestions,
  cancelQuestionSearch,
  warmQuestionSearch,
} from "$lib/services/question-search";

describe("question search service", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("uses the dedicated model warm-up command", async () => {
    invokeMock.mockResolvedValue(undefined);

    await expect(warmQuestionSearch()).resolves.toBeUndefined();
    expect(invokeMock).toHaveBeenCalledWith("warm_question_search", undefined);
  });

  it("keeps visible search requests on the search command", async () => {
    const response = {
      query: "polity",
      results: [],
      totalMatches: 0,
      searchedQuestions: 0,
    };
    invokeMock.mockResolvedValue(response);

    await expect(searchQuestions("polity", ["prelims-gs1"])).resolves.toBe(
      response,
    );
    expect(invokeMock).toHaveBeenCalledWith("search_questions", {
      args: { query: "polity", sections: ["prelims-gs1"] },
      onProgress: expect.any(Object),
    });
  });
  it("forwards progress only until completion and includes request identity", async () => {
    let finish!: (value: unknown) => void;
    invokeMock.mockImplementationOnce(
      () => new Promise((resolve) => (finish = resolve)),
    );
    const onProgress = vi.fn();
    const pending = searchQuestions("water", [], {
      clientId: "dialog",
      requestId: 3,
      onProgress,
    });
    const payload = invokeMock.mock.calls[0][1];
    expect(payload.args).toEqual({
      query: "water",
      sections: [],
      clientId: "dialog",
      requestId: 3,
    });
    const preview = {
      query: "water",
      results: [],
      totalMatches: 0,
      semanticStatus: "pending",
    };
    payload.onProgress.onmessage(preview);
    expect(onProgress).toHaveBeenCalledWith(preview);
    finish({ ...preview, semanticStatus: "available" });
    await pending;
    payload.onProgress.onmessage(preview);
    expect(onProgress).toHaveBeenCalledTimes(1);
  });

  it("cancels superseded work independently of search completion", async () => {
    invokeMock.mockResolvedValue(undefined);
    await cancelQuestionSearch("dialog", 4);
    expect(invokeMock).toHaveBeenCalledWith("cancel_question_search", {
      args: { clientId: "dialog", requestId: 4 },
    });
  });
});
