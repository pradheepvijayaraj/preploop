import { afterEach, describe, expect, it, vi } from "vitest";
import { SESSION_LOAD_TIMEOUT_MS } from "$lib/constants/timer";
import {
  LOADING_FAILURE_MESSAGE,
  markUninterruptible,
  withLoadingTimeout,
} from "$lib/services/loading-timeout";

describe("withLoadingTimeout", () => {
  afterEach(() => vi.useRealTimers());

  it("still rejects slow read-only operations", async () => {
    vi.useFakeTimers();
    const result = withLoadingTimeout(new Promise<never>(() => {}));
    const rejection = expect(result).rejects.toThrow(LOADING_FAILURE_MESSAGE);

    await vi.advanceTimersByTimeAsync(SESSION_LOAD_TIMEOUT_MS);

    await rejection;
  });

  it("awaits an uninterruptible mutation instead of reporting a false timeout", async () => {
    vi.useFakeTimers();
    let complete!: (value: string) => void;
    const mutation = markUninterruptible(
      new Promise<string>((resolve) => {
        complete = resolve;
      }),
    );
    const result = withLoadingTimeout(mutation);

    await vi.advanceTimersByTimeAsync(SESSION_LOAD_TIMEOUT_MS * 2);
    complete("synced");

    await expect(result).resolves.toBe("synced");
  });
});
