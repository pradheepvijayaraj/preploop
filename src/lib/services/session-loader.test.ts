import { beforeEach, describe, expect, it, vi } from "vitest";
import type { LoadedSessionData } from "$lib/services/session-page";

const { loadSessionDataMock } = vi.hoisted(() => ({
  loadSessionDataMock: vi.fn(),
}));

vi.mock("$lib/services/session-page", () => ({
  loadSessionData: loadSessionDataMock,
}));

import { loadSession } from "$lib/services/session-loader";

function sessionData(mode: "test" | "practice"): LoadedSessionData {
  return {
    attempt: {
      id: "attempt-id",
      bankId: "bank-id",
      mode,
      status: "in_progress",
      duration: 600,
      timeRemaining: 500,
      startedAt: 1,
    },
    questions: [],
    answers: new Map(),
    flags: new Set(),
  };
}

describe("loadSession", () => {
  beforeEach(() => loadSessionDataMock.mockReset());

  it("returns data when the route matches the persisted attempt mode", async () => {
    const data = sessionData("test");
    loadSessionDataMock.mockResolvedValue({ data });

    await expect(loadSession("attempt-id", "test")).resolves.toEqual({ data });
  });

  it("redirects to the persisted mode when the route mode is wrong", async () => {
    loadSessionDataMock.mockResolvedValue({ data: sessionData("test") });

    await expect(loadSession("attempt-id", "practice")).resolves.toEqual({
      redirectTo: "/test/attempt-id",
    });
  });

  it("preserves redirects for completed attempts", async () => {
    loadSessionDataMock.mockResolvedValue({
      redirectTo: "/results/attempt-id",
    });

    await expect(loadSession("attempt-id", "test")).resolves.toEqual({
      redirectTo: "/results/attempt-id",
    });
  });
});
