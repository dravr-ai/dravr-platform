// ABOUTME: The focus/idle contract both clients obey — when a client is allowed to talk to the server
// ABOUTME: One policy, stated outright, instead of two clients inheriting an unstated library default

/**
 * When the clients are allowed to talk to the server.
 *
 * Cloud Run runs the API with `cpu_idle = false` and instance-based billing:
 * an instance is charged for as long as it is up, and a request keeps it up.
 * A recurring poll is therefore a standing bill, and a dashboard left open in
 * a visible tab renews it forever — the idle floor never drops, no matter how
 * long ago the athlete stopped looking.
 *
 * Two cases, and only one of them was handled:
 *
 * - **Backgrounded.** React Query already skips an interval refetch while the
 *   client is unfocused, because `refetchIntervalInBackground` defaults to
 *   `false`. Web inherited that default silently; mobile bridged `AppState`
 *   into `focusManager` and wrote down why. Same intent, two mechanisms, one
 *   of them invisible — exactly the divergence shape a client-parity phase
 *   exists to end, which is why {@link QUERY_FOCUS_POLICY} states it outright
 *   on both sides rather than leaving one to a library default.
 * - **Visible but idle.** Neither client handled this, and it is the case
 *   that costs money: a tab on a second monitor is focused and visible, so
 *   every interval keeps firing against a nobody. {@link IdleWatch} is the
 *   answer — see {@link IDLE_STOP_AFTER_MS}.
 */

/**
 * How long a client goes without any user interaction before it stops talking
 * to the server.
 *
 * Five minutes. The floor is set by what "idle" must not mean: an athlete
 * reading a long reply, studying a chart, or watching a turn stream in is
 * *using* the app while producing no clicks — and any scroll, key, pointer
 * move or touch resets this, so only genuine absence reaches it. Below a
 * couple of minutes those readers would be cut off mid-sentence.
 *
 * The ceiling is set by what it must catch: the longest recurring interval
 * shipping today is 120 s (unread notifications, the admin summary tabs), so
 * five minutes stops a forgotten tab after at most three wasted polls rather
 * than the unbounded number it makes today. Longer thresholds buy nothing —
 * the tab is equally abandoned at 5 minutes and at 30 — while every extra
 * minute is billed on an instance nobody is reading.
 */
export const IDLE_STOP_AFTER_MS = 5 * 60 * 1000;

/**
 * How often to re-ask whether an OAuth grant has landed.
 *
 * A grant completes in a second browser tab (or the system browser on
 * mobile), so the screen that started it has no signal to subscribe to and
 * must ask. Five seconds is short enough to feel instant on return from the
 * provider's consent screen.
 *
 * Transient by construction: the caller stops the interval the moment a
 * connection appears, so this is the cost of the wait, never the cost of the
 * screen being open.
 */
export const PROVIDER_LINK_POLL_INTERVAL_MS = 5000;

/**
 * How often to re-ask whether a messaging channel link has landed.
 *
 * The link is completed on the athlete's *phone* (scanning a QR, tapping a
 * deep link), so the screen that offered it can only learn by asking. Three
 * seconds, because the athlete is watching this screen waiting for it to
 * advance.
 *
 * Transient by construction, exactly as {@link PROVIDER_LINK_POLL_INTERVAL_MS}
 * is: once the channel appears in the list the answer cannot change again and
 * the caller stops the interval.
 */
export const CHANNEL_LINK_POLL_INTERVAL_MS = 3000;

/**
 * The query defaults that encode the focus contract.
 *
 * Spread into each client's `QueryClient` `defaultOptions.queries`, ahead of
 * whatever else that client needs. Written out rather than inherited so that
 * a future React Query upgrade changing a default cannot silently change what
 * the clients cost.
 */
export const QUERY_FOCUS_POLICY = {
  /**
   * An unfocused client does not poll. This is React Query's default, and it
   * is the whole mechanism {@link IdleWatch} drives: going idle is expressed
   * as "not focused", so one switch governs backgrounded and idle alike.
   */
  refetchIntervalInBackground: false,
  /**
   * Coming back from an offline stretch is worth one round trip — the cached
   * screen the athlete is looking at may be hours stale.
   */
  refetchOnReconnect: true,
} as const;

/** Everything {@link IdleWatch} needs to run. */
export interface IdleWatchOptions {
  /**
   * Called when the client crosses from active to idle: stop the recurring
   * polls and drop any open stream.
   */
  onIdle: () => void;
  /** Called when an interaction brings the client back. */
  onActive: () => void;
  /** Override the threshold. Defaults to {@link IDLE_STOP_AFTER_MS}. */
  idleAfterMs?: number;
  /**
   * Timer functions, so a test can drive this with fake timers and a host can
   * supply its own (React Native's `setTimeout` returns a different handle
   * type than the DOM's).
   */
  setTimer?: (fn: () => void, ms: number) => unknown;
  /** Cancel a timer started by `setTimer`. */
  clearTimer?: (handle: unknown) => void;
}

/**
 * Stops a client talking to the server once nobody is driving it, and starts
 * it again on the next interaction.
 *
 * Deliberately platform-free: it knows nothing about DOM events, `AppState`,
 * or React Query. Each client feeds it interactions from whatever its
 * platform calls an interaction, and binds `onIdle` / `onActive` to
 * `focusManager.setFocused(false | true)`. That keeps one threshold and one
 * state machine for both, with only the event sources differing — which is
 * the part that genuinely cannot be shared.
 *
 * The watch starts active: a client is created because somebody opened it.
 */
export class IdleWatch {
  readonly #options: IdleWatchOptions;
  readonly #idleAfterMs: number;
  readonly #setTimer: (fn: () => void, ms: number) => unknown;
  readonly #clearTimer: (handle: unknown) => void;
  #handle: unknown = null;
  #idle = false;
  #stopped = false;
  #busy = 0;

  constructor(options: IdleWatchOptions) {
    this.#options = options;
    this.#idleAfterMs = options.idleAfterMs ?? IDLE_STOP_AFTER_MS;
    this.#setTimer = options.setTimer ?? ((fn, ms) => setTimeout(fn, ms));
    this.#clearTimer =
      options.clearTimer ?? (handle => clearTimeout(handle as ReturnType<typeof setTimeout>));
    this.#arm();
  }

  /** `true` while the client is stopped for idleness. */
  get isIdle(): boolean {
    return this.#idle;
  }

  /**
   * Record that a human did something. Resumes a stopped client and pushes
   * the idle deadline out.
   *
   * Safe to call on every pointer move: the only work per call is resetting
   * one timer, and `onActive` fires solely on the idle → active edge.
   */
  noteInteraction(): void {
    if (this.#stopped) return;
    if (this.#idle) {
      this.#idle = false;
      this.#options.onActive();
    }
    this.#arm();
  }

  /**
   * Mark the start of work the athlete is waiting on, and return the release.
   *
   * A streaming turn is activity even though the athlete is not touching
   * anything: they asked a question and are watching for the answer. Without
   * this, a tool-heavy turn that outruns the threshold would be aborted and
   * the tokens already spent on it thrown away — the client would be
   * punishing the athlete for the model being slow.
   *
   * Held as a count, not a flag, so two concurrent turns cannot have the
   * first one to finish release the second. The deadline is re-armed on
   * release, so the threshold measures idleness *after* the work, not during
   * it.
   */
  holdWhileBusy(): () => void {
    this.#busy += 1;
    this.#arm();
    let released = false;
    return () => {
      if (released) return;
      released = true;
      this.#busy = Math.max(0, this.#busy - 1);
      this.#arm();
    };
  }

  /**
   * Go idle now, without waiting out the threshold — what a client calls when
   * its platform says the app is no longer visible at all.
   */
  suspend(): void {
    if (this.#stopped) return;
    this.#disarm();
    this.#goIdle();
  }

  /** Tear the watch down. It reports and does nothing further. */
  stop(): void {
    this.#stopped = true;
    this.#disarm();
  }

  #arm(): void {
    this.#disarm();
    // Work the athlete is waiting on holds the client active with no timer at
    // all: a turn that takes longer than the threshold is slow, not idle.
    if (this.#busy > 0) return;
    this.#handle = this.#setTimer(() => {
      this.#handle = null;
      this.#goIdle();
    }, this.#idleAfterMs);
  }

  #disarm(): void {
    if (this.#handle !== null) {
      this.#clearTimer(this.#handle);
      this.#handle = null;
    }
  }

  #goIdle(): void {
    if (this.#idle) return;
    this.#idle = true;
    this.#options.onIdle();
  }
}
