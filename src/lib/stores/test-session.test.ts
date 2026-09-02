import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Question } from "$lib/types";

const serviceMocks = vi.hoisted(() => ({
  saveAnswer: vi.fn(),
  toggleFlag: vi.fn(),
  updateTimeRemaining: vi.fn(),
  pauseTest: vi.fn(),
  resumeTest: vi.fn(),
  logError: vi.fn(),
}));

vi.mock("$lib/services/test-session", () => ({
  saveAnswer: serviceMocks.saveAnswer,
  toggleFlag: serviceMocks.toggleFlag,
  updateTimeRemaining: serviceMocks.updateTimeRemaining,
  pauseTest: serviceMocks.pauseTest,
  resumeTest: serviceMocks.resumeTest,
}));
vi.mock("$lib/services/logger", () => ({ logError: serviceMocks.logError }));

import {
  clearTestSession,
  flushPendingSaves,
  getAnswer,
  getNavigationInfo,
  getSubmissionSnapshot,
  getTestSessionState,
  initTestSession,
  isTimerExpired,
  nextQuestion,
  pause,
  resume,
  saveAnswer,
  setSubmitting,
  toggleCurrentFlag,
} from "$lib/stores/test-session.svelte";

const questions: Question[] = [
  {
    id: "q1",
    type: "single-choice",
    question: "Question 1",
    options: [
      { id: "a", text: "A" },
      { id: "b", text: "B" },
    ],
    correctAnswers: ["a"],
    explanation: "",
    marks: 2,
    negativeMarks: 0.667,
  },
  {
    id: "q2",
    type: "single-choice",
    question: "Question 2",
    options: [],
    correctAnswers: ["a"],
    explanation: "",
    marks: 2,
    negativeMarks: 0.667,
  },
];

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

describe("test session store", () => {
  beforeEach(() => {
    clearTestSession(false);
    vi.clearAllMocks();
    serviceMocks.saveAnswer.mockResolvedValue(undefined);
    serviceMocks.toggleFlag.mockResolvedValue(true);
    serviceMocks.updateTimeRemaining.mockResolvedValue(undefined);
    serviceMocks.pauseTest.mockResolvedValue(undefined);
    serviceMocks.resumeTest.mockResolvedValue(undefined);
  });

  afterEach(() => clearTestSession(false));

  it("hydrates state and keeps navigation within bounds", () => {
    initTestSession(
      "attempt-id",
      "bank-id",
      "practice",
      questions,
      600,
      0,
      new Map([["q1", "a"]]),
      new Set(["q2"]),
    );

    expect(getTestSessionState().answers.get("q1")).toBe("a");
    expect(getTestSessionState().flags.has("q2")).toBe(true);
    expect(getNavigationInfo()).toEqual({
      current: 1,
      total: 2,
      canNext: true,
      canPrevious: false,
    });

    nextQuestion();
    nextQuestion();
    expect(getTestSessionState().currentIndex).toBe(1);
  });

  it("rolls back a failed optimistic answer save", async () => {
    serviceMocks.saveAnswer.mockRejectedValueOnce(new Error("write failed"));
    initTestSession("attempt-id", "bank-id", "practice", questions, 600, 0);

    await expect(saveAnswer("a")).rejects.toThrow("write failed");

    expect(getAnswer("q1")).toBeNull();
  });

  it("serializes same-question saves without clobbering a newer answer", async () => {
    const firstWrite = deferred<void>();
    serviceMocks.saveAnswer
      .mockImplementationOnce(() => firstWrite.promise)
      .mockResolvedValueOnce(undefined);
    initTestSession("attempt-id", "bank-id", "practice", questions, 600, 0);

    const firstSave = saveAnswer("a");
    const secondSave = saveAnswer("b");

    expect(getAnswer("q1")).toBe("b");
    expect(serviceMocks.saveAnswer).toHaveBeenCalledTimes(1);

    firstWrite.reject(new Error("first write failed"));
    await expect(firstSave).rejects.toThrow("first write failed");
    await expect(secondSave).resolves.toBeUndefined();

    expect(serviceMocks.saveAnswer).toHaveBeenCalledTimes(2);
    expect(getAnswer("q1")).toBe("b");
  });

  it("waits for pending saves and reports failures before submission", async () => {
    const write = deferred<void>();
    serviceMocks.saveAnswer.mockImplementationOnce(() => write.promise);
    initTestSession("attempt-id", "bank-id", "practice", questions, 600, 0);

    const saving = saveAnswer("a");
    const flushing = flushPendingSaves();
    write.reject(new Error("write failed"));

    await expect(saving).rejects.toThrow("write failed");
    await expect(flushing).rejects.toThrow("1 answer(s) failed to save");
  });

  it("rejects new answer mutations after submission is locked", async () => {
    initTestSession("attempt-id", "bank-id", "test", questions, 600, 300);
    setSubmitting(true);

    await saveAnswer("a");

    expect(serviceMocks.saveAnswer).not.toHaveBeenCalled();
    expect(getAnswer("q1")).toBeNull();

    await pause();
    expect(serviceMocks.pauseTest).not.toHaveBeenCalled();
    expect(getTestSessionState().isPaused).toBe(false);

    initTestSession(
      "paused-attempt",
      "bank-id",
      "test",
      questions,
      600,
      300,
      undefined,
      undefined,
      "paused",
    );
    setSubmitting(true);
    await resume();
    expect(serviceMocks.resumeTest).not.toHaveBeenCalled();
    expect(getTestSessionState().isPaused).toBe(true);
  });

  it("does not expose another session's countdown to a pending submission", () => {
    initTestSession("old-attempt", "bank-id", "test", questions, 600, 300);
    setSubmitting(true);
    expect(getSubmissionSnapshot("old-attempt")).toEqual({
      timeRemaining: 300,
    });

    initTestSession("new-attempt", "bank-id", "test", questions, 600, 590);

    expect(getSubmissionSnapshot("old-attempt")).toBeNull();
    setSubmitting(true);
    setSubmitting(false, "old-attempt");
    expect(getTestSessionState().isSubmitting).toBe(true);
  });

  it("ignores a late flag response from a replaced session", async () => {
    const toggle = deferred<boolean>();
    serviceMocks.toggleFlag.mockImplementationOnce(() => toggle.promise);
    initTestSession("old-attempt", "bank-id", "practice", questions, 600, 0);

    const toggling = toggleCurrentFlag();
    initTestSession(
      "new-attempt",
      "bank-id",
      "practice",
      [questions[1]],
      600,
      0,
    );
    toggle.resolve(true);
    await toggling;

    expect(getTestSessionState().attemptId).toBe("new-attempt");
    expect(getTestSessionState().flags.size).toBe(0);
  });

  it("rolls back pause state when persistence fails", async () => {
    serviceMocks.pauseTest.mockRejectedValueOnce(new Error("pause failed"));
    initTestSession("attempt-id", "bank-id", "test", questions, 600, 300);

    await expect(pause()).rejects.toThrow("pause failed");

    expect(getTestSessionState().isPaused).toBe(false);
    expect(isTimerExpired()).toBe(false);
  });

  it("hydrates a persisted pause and resumes through the backend", async () => {
    initTestSession(
      "attempt-id",
      "bank-id",
      "test",
      questions,
      600,
      300,
      undefined,
      undefined,
      "paused",
    );

    expect(getTestSessionState().isPaused).toBe(true);

    await resume();

    expect(serviceMocks.resumeTest).toHaveBeenCalledWith("attempt-id");
    expect(getTestSessionState().isPaused).toBe(false);
  });
});
