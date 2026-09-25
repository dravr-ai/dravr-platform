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
 * How long the Home page waits before asking once more for recent activities
 * the server reported stale.
 *
 * A stale answer means the server served the cache and started a background
 * refresh through the provider. Fifteen seconds covers a token provider's
 * fetch, which returns in a few; a scraped provider can take longer, and the
 * page then shows what it has with its sync time rather than asking again.
 *
 * One refetch, never an interval: whatever the second answer says, the page
 * stops there, so an open Home screen costs one extra request, not a poll.
 */
export const HOME_STALE_REFETCH_DELAY_MS = 15000;

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
   * Called when the client crosses into idle — {@link IDLE_STOP_AFTER_MS}
   * without an interaction, hidden or visible: stop the recurring polls and
   * drop any open stream.
   */
  onIdle: () => void;
  /**
   * Called when the platform hides the client — a background tab, a
   * backgrounded app: stop the recurring polls now. Nothing else stops. A
   * turn still streaming keeps streaming until the idle deadline, because
   * the server finishes it whether or not anyone is reading.
   */
  onSuspend: () => void;
  /**
   * Called when the client is back: an interaction after idleness, or the
   * platform showing a hidden client again. Restart the polls.
   */
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
 * it again when somebody is.
 *
 * Deliberately platform-free: it knows nothing about DOM events, `AppState`,
 * or React Query. Each client feeds it interactions from whatever its
 * platform calls an interaction, reports visibility through
 * {@link IdleWatch.suspend} / {@link IdleWatch.resume}, and binds the
 * callbacks to `focusManager.setFocused(false | true)` plus the abort of its
 * open turn stream. That keeps one threshold and one state machine for both,
 * with only the event sources differing — which is the part that genuinely
 * cannot be shared.
 *
 * Hidden and idle are two different stops, on purpose:
 *
 * - **Hidden** (`suspend`) is an athlete who switched tabs or apps — to
 *   authorize Strava, to answer a message — and is often back within a
 *   minute. Nothing on screen is being read, so the polls stop at once. A
 *   turn in flight does not: the server finishes it whether or not anyone is
 *   still reading, so dropping the stream saves nothing and costs the athlete
 *   the reply they are about to come back for.
 * - **Idle** is {@link IDLE_STOP_AFTER_MS} without an interaction, hidden or
 *   visible. That is a client left alone, and only then is an open stream
 *   dropped — what stops a tab forgotten for an hour holding an instance warm.
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
  #hidden = false;
  #stopped = false;
  #busy = 0;
  #absences = 0;
  #onReturn: (() => void)[] = [];

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
   * Note where the athlete is as work starts, and return the question to ask
   * when it ends: were they away at any point while it ran?
   *
   * Away counts two ways — already away when the work started (a turn sent
   * from a hidden tab, a prompt queued before the athlete switched away), or
   * gone since (hidden, or idle while visible). A turn that failed while
   * nobody was looking may still have been answered, because the server
   * finishes a turn whether or not anyone is reading; one that failed in
   * front of the athlete was not lost to their absence.
   */
  trackAbsence(): () => boolean {
    const awayAtStart = this.#hidden || this.#idle;
    const departuresAtStart = this.#absences;
    return () => awayAtStart || this.#absences !== departuresAtStart;
  }

  /**
   * Record that a human did something. Resumes an idle client and pushes the
   * idle deadline out.
   *
   * Ignored while the client is hidden: nobody can be driving a client the
   * platform is not showing, so whatever reaches it then is the page's own
   * doing — a reply scrolling itself into view — and must not read as the
   * athlete coming back. Only {@link IdleWatch.resume} ends a hidden stretch.
   *
   * Safe to call on every pointer move: the only work per call is resetting
   * one timer, and `onActive` fires solely on the idle → active edge.
   */
  noteInteraction(): void {
    if (this.#stopped || this.#hidden) return;
    const returning = this.#idle;
    this.#idle = false;
    this.#arm();
    if (returning) this.#returned();
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
   * The hold covers a client somebody can see. Once it is hidden nobody is
   * watching the answer arrive, so the deadline runs again from the moment it
   * was hidden — the hold is what makes a slow turn not idle, not what makes a
   * forgotten one immortal.
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
   * The platform says the client is no longer visible: stop the polls now,
   * and let the idle deadline decide the rest.
   *
   * An unheld client keeps the deadline its last interaction set. A held one
   * had no deadline while somebody was watching; nobody is now, so its
   * deadline starts here.
   */
  suspend(): void {
    if (this.#stopped || this.#hidden) return;
    this.#hidden = true;
    // Already idle: the polls are stopped and nothing is left open.
    if (this.#idle) return;
    this.#absences += 1;
    this.#options.onSuspend();
    if (this.#handle === null) this.#arm();
  }

  /**
   * The platform shows the client again. Coming back to it is the
   * interaction that ends the absence, whether or not the deadline passed
   * while it was hidden.
   *
   * It ends an idle stretch too, hidden or not: a platform saying "shown" is
   * somebody looking, even when the hide it pairs with never reached the
   * watch.
   */
  resume(): void {
    if (this.#stopped || (!this.#hidden && !this.#idle)) return;
    this.#hidden = false;
    this.#idle = false;
    this.#arm();
    this.#returned();
  }

  /**
   * Run `work` once somebody is here: now when the client is visible and
   * active, otherwise on its return, right after `onActive` has restarted the
   * polls.
   *
   * What a turn that failed while the athlete was away queues: the re-read
   * that shows them the reply the server went on to write. Queued work runs
   * once, on the first return; a watch torn down first drops it.
   */
  whenPresent(work: () => void): void {
    if (this.#stopped) return;
    if (!this.#idle && !this.#hidden) {
      work();
      return;
    }
    this.#onReturn.push(work);
  }

  /** Tear the watch down. It reports and does nothing further. */
  stop(): void {
    this.#stopped = true;
    this.#disarm();
    this.#onReturn = [];
  }

  #arm(): void {
    this.#disarm();
    // Work the athlete is watching holds a visible client active with no
    // timer at all: a turn that takes longer than the threshold is slow, not
    // idle. Hidden, nobody is watching, and the deadline runs.
    if (this.#busy > 0 && !this.#hidden) return;
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
    // A hidden client already counted its absence when it was hidden; work
    // started while it was hidden is answered by `trackAbsence`'s start check.
    if (!this.#hidden) this.#absences += 1;
    this.#options.onIdle();
  }

  #returned(): void {
    this.#options.onActive();
    const work = this.#onReturn;
    this.#onReturn = [];
    for (const run of work) run();
  }
}
