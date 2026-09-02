import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

import {
  BackendError,
  invokeBackend,
  normalizeBackendError,
} from "$lib/services/backend";

describe("invokeBackend", () => {
  beforeEach(() => invokeMock.mockReset());

  it("owns the Tauri args envelope", async () => {
    invokeMock.mockResolvedValue("attempt-id");

    await expect(
      invokeBackend("create_test_attempt", {
        bankId: "bank-id",
        mode: "test",
      }),
    ).resolves.toBe("attempt-id");
    expect(invokeMock).toHaveBeenCalledWith("create_test_attempt", {
      args: { bankId: "bank-id", mode: "test" },
    });
  });

  it("rejects a malformed command response at the IPC boundary", async () => {
    invokeMock.mockResolvedValue({ score: 2 });

    await expect(
      invokeBackend("submit_test", { attemptId: "id" }),
    ).rejects.toThrow("Invalid response from backend command: submit_test");
  });

  it("maps the structured backend error contract to an Error instance", async () => {
    const error = normalizeBackendError({
      code: "invalid_state",
      message: "Attempt is already completed",
    }) as BackendError;

    expect(error.code).toBe("invalid_state");
    expect(error.message).toBe("Attempt is already completed");
  });
});
