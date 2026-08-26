// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The abort signal every open turn stream rides, so going idle can drop it
// ABOUTME: One controller per active stretch; a fresh one is minted when the athlete returns

/**
 * The controller the current active stretch shares.
 *
 * A turn opened while the athlete is present is aborted when they leave, and
 * a turn opened after they come back must not be — hence a new controller per
 * stretch rather than one for the life of the page.
 */
let controller = new AbortController();

/**
 * The signal to hand {@link import('@pierre/api-client').SendTurnOptions}.
 *
 * Read at send time, not cached: a caller holding a stale signal would send a
 * turn that is already aborted.
 */
export function idleSignal(): AbortSignal {
  return controller.signal;
}

/** Abort every stream opened during this active stretch. */
export function idleAbort(): void {
  controller.abort();
}

/**
 * The live idle watch, registered by {@link useIdleWatch} at the app root.
 *
 * Held here rather than passed down because the send path is several
 * components away from where the watch is mounted, and threading it through
 * every caller would be more moving parts than a module-scoped registration.
 */
let watch: { holdWhileBusy: () => () => void } | null = null;

/** Register the app's idle watch so streaming turns can hold it active. */
export function registerIdleWatch(w: { holdWhileBusy: () => () => void } | null): void {
  watch = w;
}

/**
 * Hold the client active for the length of a turn, and return the release.
 *
 * An athlete waiting on a slow tool-loop turn is not touching anything, but
 * they are not idle either. Without this hold the watch would abort the very
 * turn they are waiting for and discard the tokens already spent on it.
 */
export function holdIdleWhileBusy(): () => void {
  return watch?.holdWhileBusy() ?? (() => {});
}

/** Start a fresh stretch, so turns sent from here on are not born aborted. */
export function resetIdleAbort(): void {
  if (controller.signal.aborted) {
    controller = new AbortController();
  }
}
