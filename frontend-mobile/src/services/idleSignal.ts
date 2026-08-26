// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
// ABOUTME: The abort signal every open turn stream rides, so going idle can drop it
// ABOUTME: One controller per active stretch; a fresh one is minted when the athlete returns

/**
 * The controller the current active stretch shares.
 *
 * A turn opened while the athlete is present is aborted when they leave, and
 * a turn opened after they come back must not be — hence a new controller per
 * stretch rather than one for the life of the process.
 *
 * Deliberately a mirror of the web module of the same name: both clients open
 * the same stream through the same `sendTurn`, so they drop it the same way.
 */
let controller = new AbortController();

/**
 * The signal to hand `sendTurn`.
 *
 * Read at send time, not cached: a caller holding a stale signal would send a
 * turn that is already aborted.
 */
export function idleSignal(): AbortSignal {
  return controller.signal;
}

/**
 * The live idle watch, registered by QueryProvider at the app root.
 *
 * Mirrors the web module: the send path is several screens away from where
 * the watch is mounted.
 */
let watch: { holdWhileBusy: () => () => void } | null = null;

/** Register the app's idle watch so streaming turns can hold it active. */
export function registerIdleWatch(w: { holdWhileBusy: () => () => void } | null): void {
  watch = w;
}

/**
 * Hold the client active for the length of a turn, and return the release.
 *
 * An athlete waiting on a slow tool-loop turn is not touching the screen, but
 * they are not idle either. Without this hold the watch would abort the very
 * turn they are waiting for and discard the tokens already spent on it.
 */
export function holdIdleWhileBusy(): () => void {
  return watch?.holdWhileBusy() ?? (() => {});
}

/** Abort every stream opened during this active stretch. */
export function idleAbort(): void {
  controller.abort();
}

/** Start a fresh stretch, so turns sent from here on are not born aborted. */
export function resetIdleAbort(): void {
  if (controller.signal.aborted) {
    controller = new AbortController();
  }
}
