// @vitest-environment jsdom

import { render, screen } from "@testing-library/svelte";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import type { Question } from "$lib/types";
import QuestionNavigator from "$lib/components/question-navigator.svelte";

const question = (id: string): Question => ({
  id,
  type: "single-choice",
  question: id,
  marks: 2,
  negativeMarks: 0.667,
});

beforeAll(() => {
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  );
});

afterAll(() => vi.unstubAllGlobals());

describe("QuestionNavigator", () => {
  it("counts only questions in a filtered navigator", () => {
    render(QuestionNavigator, {
      props: {
        questions: [question("skipped")],
        currentIndex: 0,
        answers: new Map([["answered-outside-filter", "a"]]),
        flags: new Set(["flagged-outside-filter"]),
        expanded: true,
        onNavigate: vi.fn(),
      },
    });

    const legend = screen.getByRole("group", {
      name: "Question status legend",
    });
    expect(legend.textContent).toMatch(/Answered\s*0/);
    expect(legend.textContent).toMatch(/Unanswered\s*1/);
    expect(legend.textContent).toMatch(/Flagged\s*0/);
  });
});
