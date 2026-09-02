import { beforeEach, describe, expect, it, vi } from "vitest";
import type { StoredQuestionBank, TestAttempt } from "$lib/types";

const { getQuestionBankMock, getTestAttemptMock } = vi.hoisted(() => ({
  getQuestionBankMock: vi.fn(),
  getTestAttemptMock: vi.fn(),
}));

vi.mock("$lib/services/question-bank", () => ({
  getQuestionBank: getQuestionBankMock,
}));
vi.mock("$lib/services/test-session", () => ({
  getTestAttempt: getTestAttemptMock,
}));

import { loadResultContext } from "$lib/services/result-loader";

const completedAttempt: TestAttempt = {
  id: "attempt-id",
  bankId: "bank-id",
  mode: "test",
  status: "completed",
  duration: 600,
  timeRemaining: 0,
  startedAt: 1,
  completedAt: 2,
};

const bank: StoredQuestionBank = {
  id: "bank-id",
  name: "Bank",
  exam: "UPSC",
  metadata: "{}",
  totalQuestions: 1,
  difficulty: "medium",
  defaultDuration: 600,
  importedAt: 1,
};

describe("loadResultContext", () => {
  beforeEach(() => {
    getQuestionBankMock.mockReset();
    getTestAttemptMock.mockReset();
  });

  it("loads the bank for a completed attempt", async () => {
    getTestAttemptMock.mockResolvedValue(completedAttempt);
    getQuestionBankMock.mockResolvedValue(bank);

    await expect(loadResultContext("attempt-id")).resolves.toEqual({
      attempt: completedAttempt,
      bank,
    });
  });

  it("rejects result and review access while an attempt is active", async () => {
    getTestAttemptMock.mockResolvedValue({
      ...completedAttempt,
      status: "in_progress",
      completedAt: undefined,
    });

    await expect(loadResultContext("attempt-id")).rejects.toThrow(
      "Results are only available after the session is completed",
    );
    expect(getQuestionBankMock).not.toHaveBeenCalled();
  });

  it("reports a missing attempt", async () => {
    getTestAttemptMock.mockResolvedValue(null);

    await expect(loadResultContext("missing")).rejects.toThrow(
      "Test attempt not found",
    );
  });
});
