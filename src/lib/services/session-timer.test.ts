import { afterEach, describe, expect, it, vi } from "vitest";
import { SessionCountdown } from "$lib/services/session-timer";

afterEach(() => vi.useRealTimers());

describe("SessionCountdown", () => {
  it("anchors remaining time to the wall clock", () => {
    vi.useFakeTimers();
    vi.setSystemTime(0);
    const changes: number[] = [];
    const onExpire = vi.fn();
    const timer = new SessionCountdown({
      onChange: (seconds) => changes.push(seconds),
      onPersist: vi.fn(),
      onExpire,
    });
    timer.start(120);
    vi.setSystemTime(61_000);
    window.dispatchEvent(new Event("focus"));
    expect(changes.at(-1)).toBe(59);
    vi.setSystemTime(121_000);
    window.dispatchEvent(new Event("focus"));
    expect(changes.at(-1)).toBe(0);
    expect(onExpire).toHaveBeenCalledOnce();
  });

  it("starts ticking when a persisted paused countdown is resumed", () => {
    vi.useFakeTimers();
    vi.setSystemTime(0);
    const onChange = vi.fn();
    const onPersist = vi.fn();
    const timer = new SessionCountdown({
      onChange,
      onPersist,
      onExpire: vi.fn(),
    });

    timer.resume(60);
    vi.advanceTimersByTime(30_000);

    expect(onChange).toHaveBeenLastCalledWith(30);
    expect(onPersist).toHaveBeenCalledOnce();
    timer.stop();
  });
});
