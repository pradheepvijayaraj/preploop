// @vitest-environment jsdom

import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import {
  afterAll,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";
import type { QuestionSearchResponse } from "$lib/types";

const { searchQuestionsMock, cancelQuestionSearchMock } = vi.hoisted(() => ({
  searchQuestionsMock: vi.fn(),
  cancelQuestionSearchMock: vi.fn(),
}));

vi.mock("$lib/services/question-search", () => ({
  searchQuestions: searchQuestionsMock,
  cancelQuestionSearch: cancelQuestionSearchMock,
}));

import QuestionSearch from "$lib/components/question-search.svelte";

function response(
  query: string,
  matchStrength: "strong" | "related",
): QuestionSearchResponse {
  return {
    query,
    searchedQuestions: 4107,
    totalMatches: 1,
    correctedQuery: null,
    originalSpellingQuery: null,
    highlightTerms: [],
    spellingAlternatives: [],
    semanticStatus: "available",
    results: [
      {
        questionId: "upsc_2013_csat_q13",
        bankId: "upsc-2013-csat",
        bankName: "UPSC CSE Prelims CSAT 2013",
        questionNumber: 13,
        question: `${query} result`,
        options: [],
        year: 2013,
        stage: "Prelims",
        paper: "CSAT",
        section: "prelims-csat",
        mainTag: "CSAT",
        subtags: ["Logical Reasoning"],
        similarity: 1,
        matchStrength,
        lexicalMatch: matchStrength === "strong",
        semanticMatch: true,
      },
    ],
  };
}

beforeAll(() => {
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      disconnect() {}
    },
  );
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    callback(0);
    return 1;
  });
});

beforeEach(() => {
  searchQuestionsMock.mockReset();
  cancelQuestionSearchMock.mockReset().mockResolvedValue(undefined);
});
afterAll(() => vi.unstubAllGlobals());

describe("QuestionSearch", () => {
  it("waits for keyboard composition to finish before searching", async () => {
    searchQuestionsMock.mockResolvedValue(response("café", "strong"));
    render(QuestionSearch, { props: { open: true } });
    const input = screen.getByRole("textbox", { name: "Search questions" });
    await fireEvent.input(input, { target: { value: "caf" } });
    await fireEvent.compositionStart(input);
    await fireEvent.input(input, { target: { value: "café" } });
    await new Promise((resolve) => setTimeout(resolve, 200));
    expect(searchQuestionsMock).not.toHaveBeenCalled();
    await fireEvent.compositionEnd(input);
    await waitFor(() => expect(searchQuestionsMock).toHaveBeenCalledTimes(1));
    expect(searchQuestionsMock).toHaveBeenLastCalledWith(
      "café",
      [],
      expect.objectContaining({ onProgress: expect.any(Function) }),
    );
  });

  it("retries original spelling without changing the input and resumes normal search on edit", async () => {
    let finishRetry!: (value: QuestionSearchResponse) => void;
    searchQuestionsMock
      .mockResolvedValueOnce({
        ...response("silvre notice", "strong"),
        correctedQuery: "silver notice",
        originalSpellingQuery: '"silvre" "notice"',
      })
      .mockImplementationOnce(
        () => new Promise((resolve) => (finishRetry = resolve)),
      )
      .mockResolvedValueOnce(response("silvre notices", "strong"));
    render(QuestionSearch, { props: { open: true } });
    const input = screen.getByRole("textbox", { name: "Search questions" });
    await fireEvent.input(input, { target: { value: "silvre notice" } });
    expect(await screen.findByText("“silver notice”")).toBeTruthy();
    await fireEvent.click(
      screen.getByRole("button", {
        name: "Search for “silvre notice” without spelling correction",
      }),
    );
    expect((input as HTMLInputElement).value).toBe("silvre notice");
    expect(document.activeElement).toBe(input);
    await waitFor(() =>
      expect(searchQuestionsMock).toHaveBeenLastCalledWith(
        '"silvre" "notice"',
        [],
        expect.objectContaining({ onProgress: expect.any(Function) }),
      ),
    );
    // Retrying must not collapse the correction row above the old results.
    expect(screen.getByText("“silver notice”")).toBeTruthy();
    expect(screen.getByText("silvre notice result")).toBeTruthy();
    expect(
      (
        screen.getByRole("button", {
          name: "Search for “silvre notice” without spelling correction",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
    expect(screen.queryByRole("heading", { name: "No results" })).toBeNull();

    finishRetry({
      ...response('"silvre" "notice"', "related"),
      totalMatches: 0,
      results: [],
    });
    expect(
      await screen.findByRole("heading", { name: "No results" }),
    ).toBeTruthy();
    expect(screen.queryByText("“silver notice”")).toBeNull();
    expect(screen.queryByText("silvre notice result")).toBeNull();
    expect((input as HTMLInputElement).value).toBe("silvre notice");
    expect(screen.getByRole("status").textContent).toContain("0 matches");
    expect(
      screen.queryByRole("button", {
        name: "Search for “silvre notice” without spelling correction",
      }),
    ).toBeNull();
    await fireEvent.input(input, { target: { value: "silvre notices" } });
    expect(await screen.findByText("silvre notices result")).toBeTruthy();
    expect(searchQuestionsMock).toHaveBeenLastCalledWith(
      "silvre notices",
      [],
      expect.any(Object),
    );
  });

  it("starts the latest query before old inference finishes and rejects stale progress", async () => {
    let finishOld!: (value: QuestionSearchResponse) => void;
    searchQuestionsMock
      .mockImplementationOnce(
        () => new Promise((resolve) => (finishOld = resolve)),
      )
      .mockResolvedValueOnce(response("silver notice", "strong"));
    render(QuestionSearch, {
      props: { open: true, sections: ["prelims-gs1"] },
    });
    const input = screen.getByRole("textbox", { name: "Search questions" });
    await fireEvent.input(input, { target: { value: "silver" } });
    await waitFor(() => expect(searchQuestionsMock).toHaveBeenCalledTimes(1));
    const oldControls = searchQuestionsMock.mock.calls[0][2];
    await fireEvent.input(input, { target: { value: "silver notice" } });
    expect(await screen.findByText("silver notice result")).toBeTruthy();
    expect(searchQuestionsMock).toHaveBeenCalledTimes(2);
    expect(cancelQuestionSearchMock).toHaveBeenCalledWith(
      oldControls.clientId,
      expect.any(Number),
    );
    oldControls.onProgress(response("silver", "strong"));
    finishOld(response("silver", "strong"));
    await waitFor(() => expect(screen.queryByText("silver result")).toBeNull());
    expect(screen.getByText("silver notice result")).toBeTruthy();
  });

  it("shows keyword progress immediately and retains it if related search fails", async () => {
    let fail!: (error: Error) => void;
    searchQuestionsMock.mockImplementationOnce(
      () => new Promise((_, reject) => (fail = reject)),
    );
    render(QuestionSearch, { props: { open: true } });
    await fireEvent.input(screen.getByRole("textbox"), {
      target: { value: "water" },
    });
    await waitFor(() => expect(searchQuestionsMock).toHaveBeenCalledTimes(1));
    searchQuestionsMock.mock.calls[0][2].onProgress({
      ...response("water", "strong"),
      semanticStatus: "pending",
    });
    expect(await screen.findByText("water result")).toBeTruthy();
    expect(screen.getByRole("status").textContent).toContain(
      "Updating results",
    );
    fail(new Error("inference failed"));
    expect(
      await screen.findByText(
        "Related search unavailable. Showing keyword matches.",
      ),
    ).toBeTruthy();
    expect(screen.getByText("water result")).toBeTruthy();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("offers ambiguous spellings without rewriting the query until selected", async () => {
    searchQuestionsMock
      .mockResolvedValueOnce({
        ...response("Siver", "related"),
        results: [],
        totalMatches: 0,
        spellingAlternatives: ["river", "silver"],
        semanticStatus: "notRequested",
      })
      .mockResolvedValueOnce(response("Silver", "strong"));
    render(QuestionSearch, { props: { open: true } });
    const input = screen.getByRole("textbox") as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "Siver" } });
    expect(await screen.findByText("Did you mean")).toBeTruthy();
    expect(input.value).toBe("Siver");
    await fireEvent.click(screen.getByRole("button", { name: "“Silver”" }));
    expect(input.value).toBe("Silver");
    expect(document.activeElement).toBe(input);
    expect(await screen.findByText("Silver result")).toBeTruthy();
  });

  it("keeps the visible question anchored when semantic results change its position", async () => {
    let finish!: (value: QuestionSearchResponse) => void;
    searchQuestionsMock.mockImplementationOnce(
      () => new Promise((resolve) => (finish = resolve)),
    );
    render(QuestionSearch, { props: { open: true } });
    await fireEvent.input(screen.getByRole("textbox"), {
      target: { value: "water" },
    });
    await waitFor(() => expect(searchQuestionsMock).toHaveBeenCalledTimes(1));
    searchQuestionsMock.mock.calls[0][2].onProgress({
      ...response("water", "strong"),
      semanticStatus: "pending",
    });
    await screen.findByText("water result");
    const scroller = document.querySelector<HTMLElement>(
      "#question-search-results .overflow-y-auto",
    )!;
    await waitFor(() => expect(scroller.scrollTop).toBe(0));
    scroller.scrollTop = 240;
    const anchor = scroller.querySelector<HTMLElement>("[data-question-id]")!;
    const rect = (top: number) => ({ top, bottom: top + 100 }) as DOMRect;
    vi.spyOn(scroller, "getBoundingClientRect").mockReturnValue(rect(0));
    vi.spyOn(anchor, "getBoundingClientRect")
      .mockReturnValueOnce(rect(-20))
      .mockReturnValueOnce(rect(-20))
      .mockReturnValue(rect(100));
    finish(response("water", "strong"));
    await waitFor(() => expect(scroller.scrollTop).toBe(360));
    expect(screen.getByText("water result")).toBeTruthy();
  });

  it("does not replace completed results with a late preview", async () => {
    searchQuestionsMock.mockResolvedValueOnce(response("water", "strong"));
    render(QuestionSearch, { props: { open: true } });
    await fireEvent.input(screen.getByRole("textbox"), {
      target: { value: "water" },
    });
    await screen.findByText("water result");
    searchQuestionsMock.mock.calls[0][2].onProgress({
      ...response("water", "strong"),
      results: [],
      semanticStatus: "pending",
    });
    await waitFor(() => expect(screen.getByText("water result")).toBeTruthy());
    expect(screen.queryByRole("heading", { name: "Searching…" })).toBeNull();
  });

  it("ignores an old failure after the query changes", async () => {
    let failOld!: (error: Error) => void;
    searchQuestionsMock
      .mockImplementationOnce(
        () => new Promise((_, reject) => (failOld = reject)),
      )
      .mockResolvedValueOnce(response("silver n", "strong"));
    render(QuestionSearch, { props: { open: true } });
    const input = screen.getByRole("textbox", { name: "Search questions" });
    await fireEvent.input(input, { target: { value: "silver" } });
    await waitFor(() => expect(searchQuestionsMock).toHaveBeenCalledTimes(1));
    await fireEvent.input(input, { target: { value: "silver n" } });
    failOld(new Error("stale failure"));
    expect(await screen.findByText("silver n result")).toBeTruthy();
    expect(screen.queryByText("stale failure")).toBeNull();
  });

  it("does not globally disable pointer input while search is open", async () => {
    const view = render(QuestionSearch);

    await fireEvent.click(
      screen.getByRole("button", { name: "Search All Papers" }),
    );
    expect(
      await screen.findByRole("textbox", { name: "Search questions" }),
    ).toBeTruthy();

    // Bits UI applies its body lock after the dialog DOM has settled.
    await new Promise((resolve) => setTimeout(resolve, 30));
    expect(document.body.style.pointerEvents).not.toBe("none");

    await fireEvent.keyDown(document, { key: "Escape" });
    await waitFor(() =>
      expect(
        screen.queryByRole("textbox", { name: "Search questions" }),
      ).toBeNull(),
    );
    expect(document.body.style.pointerEvents).not.toBe("none");

    await fireEvent.click(
      screen.getByRole("button", { name: "Search All Papers" }),
    );
    expect(
      await screen.findByRole("textbox", { name: "Search questions" }),
    ).toBeTruthy();

    view.unmount();
  });

  it("dismisses on an outside pointer interaction and can immediately reopen", async () => {
    const view = render(QuestionSearch);

    await fireEvent.click(
      screen.getByRole("button", { name: "Search All Papers" }),
    );
    expect(
      await screen.findByRole("textbox", { name: "Search questions" }),
    ).toBeTruthy();

    const overlay = document.querySelector<HTMLElement>(
      '[data-slot="dialog-overlay"]',
    );
    expect(overlay).toBeTruthy();
    // DismissibleLayer installs its document listener after the portal settles
    // and validates that the pointer coordinates are outside the content rect.
    await new Promise((resolve) => setTimeout(resolve, 30));
    await fireEvent.pointerDown(overlay!, {
      button: 0,
      clientX: 10,
      clientY: 10,
      pointerType: "mouse",
    });

    await waitFor(() =>
      expect(
        screen.queryByRole("textbox", { name: "Search questions" }),
      ).toBeNull(),
    );
    expect(document.body.style.pointerEvents).not.toBe("none");

    await fireEvent.click(
      screen.getByRole("button", { name: "Search All Papers" }),
    );
    expect(
      await screen.findByRole("textbox", { name: "Search questions" }),
    ).toBeTruthy();

    view.unmount();
  });

  it("groups confidence tiers and shows the source question number", async () => {
    searchQuestionsMock.mockResolvedValue(response("water", "strong"));
    render(QuestionSearch, { props: { open: true } });

    const input = screen.getByRole("textbox", { name: "Search questions" });
    await fireEvent.input(input, { target: { value: "water" } });

    expect(await screen.findByText("Strong matches")).toBeTruthy();
    expect(screen.getByText("Q 13")).toBeTruthy();
    expect(screen.queryByText("Q 1")).toBeNull();
  });

  it("resets the result viewport when a new response is accepted", async () => {
    searchQuestionsMock
      .mockResolvedValueOnce(response("water", "strong"))
      .mockResolvedValueOnce(response("forest", "related"));
    render(QuestionSearch, { props: { open: true } });
    const input = screen.getByRole("textbox", { name: "Search questions" });

    await fireEvent.input(input, { target: { value: "water" } });
    await screen.findByText("water result");
    const scroller = document.querySelector<HTMLElement>(
      "#question-search-results .overflow-y-auto",
    );
    expect(scroller).toBeTruthy();
    if (!scroller) return;
    scroller.scrollTop = 420;

    await fireEvent.input(input, { target: { value: "forest" } });
    await screen.findByText("forest result");
    await waitFor(() => expect(scroller.scrollTop).toBe(0));
    expect(screen.getByText("Related results")).toBeTruthy();
  });
});
