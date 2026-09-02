import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeBackendMock } = vi.hoisted(() => ({
  invokeBackendMock: vi.fn(),
}));

vi.mock("$lib/services/backend", () => ({
  invokeBackend: invokeBackendMock,
}));

import { submitTest } from "$lib/services/test-session";

describe("test-session service", () => {
  beforeEach(() => {
    invokeBackendMock.mockReset();
    invokeBackendMock.mockResolvedValue({ score: 10, maxScore: 20 });
  });

  it("submits the authoritative countdown value with a timed attempt", async () => {
    await submitTest("attempt-id", 417);

    expect(invokeBackendMock).toHaveBeenCalledWith("submit_test", {
      attemptId: "attempt-id",
      timeRemaining: 417,
    });
  });

  it("omits countdown state for untimed practice submission", async () => {
    await submitTest("attempt-id");

    expect(invokeBackendMock).toHaveBeenCalledWith("submit_test", {
      attemptId: "attempt-id",
    });
  });
});
