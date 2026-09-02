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
});
