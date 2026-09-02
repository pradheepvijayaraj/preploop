import {
  TIMER_PERSIST_INTERVAL_SECONDS,
  TIMER_TICK_MS,
} from "$lib/constants/timer";

interface CountdownCallbacks {
  onChange: (seconds: number) => void;
  onPersist: () => void;
  onExpire: () => void;
}

/** Wall-clock countdown lifecycle, independent from Svelte session state. */
export class SessionCountdown {
  private interval: ReturnType<typeof setInterval> | null = null;
  private endsAt: number | null = null;
  private remaining = 0;
  private lastPersisted = 0;

  constructor(private readonly callbacks: CountdownCallbacks) {}

  start(seconds: number): void {
    this.stop();
    this.remaining = seconds;
    this.lastPersisted = seconds;
    this.endsAt = Date.now() + seconds * 1000;
    window.addEventListener("focus", this.reconcile);
    document.addEventListener("visibilitychange", this.reconcile);
    this.interval = setInterval(this.reconcile, TIMER_TICK_MS);
  }

  pause(): number {
    this.reconcile();
    this.endsAt = null;
    return this.remaining;
  }

  resume(seconds: number): void {
    this.remaining = seconds;
    // A persisted paused session can be hydrated without `start()` ever
    // running. Establish a fresh persistence checkpoint so periodic writes
    // resume after the configured interval instead of comparing against zero.
    if (this.interval === null) this.lastPersisted = seconds;
    this.endsAt = Date.now() + seconds * 1000;
    if (this.interval === null) {
      window.addEventListener("focus", this.reconcile);
      document.addEventListener("visibilitychange", this.reconcile);
      this.interval = setInterval(this.reconcile, TIMER_TICK_MS);
    }
  }

  stop(): void {
    if (this.interval) clearInterval(this.interval);
    this.interval = null;
    this.endsAt = null;
    if (typeof window !== "undefined")
      window.removeEventListener("focus", this.reconcile);
    if (typeof document !== "undefined")
      document.removeEventListener("visibilitychange", this.reconcile);
  }

  private reconcile = (): void => {
    if (this.endsAt === null) return;
    this.remaining = Math.max(0, Math.ceil((this.endsAt - Date.now()) / 1000));
    this.callbacks.onChange(this.remaining);
    if (
      this.remaining > 0 &&
      this.lastPersisted - this.remaining >= TIMER_PERSIST_INTERVAL_SECONDS
    ) {
      this.lastPersisted = this.remaining;
      this.callbacks.onPersist();
    }
    if (this.remaining <= 0) {
      this.stop();
      this.callbacks.onExpire();
    }
  };
}
