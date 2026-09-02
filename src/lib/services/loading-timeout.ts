import { SESSION_LOAD_TIMEOUT_MS } from "$lib/constants/timer";

export const LOADING_FAILURE_MESSAGE =
  "Failed. Try again. Restart if it keeps failing.";

const UNINTERRUPTIBLE_OPERATION = Symbol("uninterruptibleOperation");

type UninterruptiblePromise<T> = Promise<T> & {
  [UNINTERRUPTIBLE_OPERATION]: true;
};

/**
 * Mark a mutation that cannot be cancelled after it crosses the IPC boundary.
 * A caller may still use the shared loading helper, but it will truthfully
 * await completion instead of reporting a timeout while writes continue.
 */
export function markUninterruptible<T>(
  operation: Promise<T>,
): UninterruptiblePromise<T> {
  Object.defineProperty(operation, UNINTERRUPTIBLE_OPERATION, {
    value: true,
  });
  return operation as UninterruptiblePromise<T>;
}

function isUninterruptible<T>(operation: Promise<T>): boolean {
  return UNINTERRUPTIBLE_OPERATION in operation;
}

/** Reject a slow loading operation after the shared UI loading deadline. */
export async function withLoadingTimeout<T>(operation: Promise<T>): Promise<T> {
  if (isUninterruptible(operation)) return operation;

  let timeoutId: ReturnType<typeof setTimeout> | null = null;
  try {
    return await Promise.race([
      operation,
      new Promise<never>((_, reject) => {
        timeoutId = setTimeout(() => {
          reject(new Error(LOADING_FAILURE_MESSAGE));
        }, SESSION_LOAD_TIMEOUT_MS);
      }),
    ]);
  } finally {
    if (timeoutId !== null) clearTimeout(timeoutId);
  }
}
