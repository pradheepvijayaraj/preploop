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

const { searchQuestionsMock } = vi.hoisted(() => ({
  searchQuestionsMock: vi.fn(),
}));

vi.mock("$lib/services/question-search", () => ({
  searchQuestions: searchQuestionsMock,
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

beforeEach(() => searchQuestionsMock.mockReset());
afterAll(() => vi.unstubAllGlobals());

describe("QuestionSearch", () => {
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
