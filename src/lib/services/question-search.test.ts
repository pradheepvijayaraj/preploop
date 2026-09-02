import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

import {
  searchQuestions,
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
    });
  });
});
