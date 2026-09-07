// @vitest-environment jsdom

import { render, screen } from "@testing-library/svelte";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import type { Question } from "$lib/types";
import QuestionCard from "$lib/components/question-card.svelte";

const baseQuestion: Question = {
  id: "question-id",
  type: "numerical",
  question: "What is 2 + 2?",
  correctAnswers: ["4"],
  explanation: "",
  marks: 2,
  negativeMarks: 0.667,
};

beforeAll(() => {
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  );
  Object.defineProperty(HTMLElement.prototype, "scrollTo", {
    configurable: true,
    value: vi.fn(),
  });
});

afterAll(() => vi.unstubAllGlobals());

describe("QuestionCard", () => {
  it("associates a label with numerical answers", () => {
    render(QuestionCard, {
      props: {
        question: baseQuestion,
        index: 0,
        total: 1,
        answer: null,
        isFlagged: false,
        onAnswer: vi.fn(),
        onToggleFlag: vi.fn(),
      },
    });

    expect(screen.getByRole("spinbutton", { name: "Answer" })).toBeTruthy();
  });

  it("associates a label with open-ended practice notes", () => {
    render(QuestionCard, {
      props: {
        question: {
          ...baseQuestion,
          type: "fill-blank",
          correctAnswers: ["__open__"],
        },
        index: 0,
        total: 1,
        answer: null,
        isFlagged: false,
        onAnswer: vi.fn(),
        onToggleFlag: vi.fn(),
      },
    });

    expect(
      screen.getByRole("textbox", { name: "Practice answer notes" }),
    ).toBeTruthy();
  });

  it("names the single-choice answer group", () => {
    render(QuestionCard, {
      props: {
        question: {
          ...baseQuestion,
          type: "single-choice",
          options: [
            { id: "a", text: "Three" },
            { id: "b", text: "Four" },
          ],
          correctAnswers: ["b"],
        },
        index: 0,
        total: 1,
        answer: null,
        isFlagged: false,
        onAnswer: vi.fn(),
        onToggleFlag: vi.fn(),
      },
    });

    expect(screen.getByRole("group", { name: "Answer choices" })).toBeTruthy();
  });

  it("renders official option-table headers and cells in the answer pane", () => {
    const { container } = render(QuestionCard, {
      props: {
        question: {
          ...baseQuestion,
          type: "single-choice",
          question: "Which pair is correctly matched?",
          options: [
            {
              id: "a",
              text: "Abyssinian Plateau: Arabia",
              cells: [
                { label: "Geographical Feature", text: "Abyssinian Plateau" },
                { label: "Region", text: "Arabia" },
              ],
            },
          ],
          correctAnswers: ["a"],
        },
        index: 0,
        total: 1,
        answer: null,
        isFlagged: false,
        onAnswer: vi.fn(),
        onToggleFlag: vi.fn(),
      },
    });

    const answerPane = container.querySelector(".question-card__pane--answer");
    expect(answerPane?.textContent).toContain("Geographical Feature");
    expect(answerPane?.textContent).toContain("Region");
    expect(answerPane?.textContent).toContain("Abyssinian Plateau");
    expect(answerPane?.textContent).toContain("Arabia");
    expect(
      container.querySelectorAll(".answer-option-table__separator"),
    ).toHaveLength(2);
    expect(
      container.querySelector(".answer-option-table__header")?.textContent,
    ).not.toContain(":");
    expect(
      container.querySelector(".question-card__pane--prompt")?.textContent,
    ).not.toContain("Geographical Feature");
  });
});
