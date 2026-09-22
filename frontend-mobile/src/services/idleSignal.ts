// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
// ABOUTME: The abort signal every open turn stream rides, so going idle can drop it
// ABOUTME: One controller per active stretch; a turn lost while the athlete was away is re-read on return

import type { IdleWatch } from '@pierre/shared-constants';

/**
 * The controller the current active stretch shares.
 *
 * A turn opened while the athlete is present is aborted when the app goes
 * idle, and a turn opened after they come back must not be — hence a new
 * controller per stretch rather than one for the life of the process.
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

/** The part of the idle watch the send path talks to. */
type SendPathWatch = Pick<IdleWatch, 'holdWhileBusy' | 'whenPresent' | 'trackAbsence'>;

/**
 * The live idle watch, registered by QueryProvider at the app root.
 *
 * Mirrors the web module: the send path is several screens away from where
 * the watch is mounted.
 */
let watch: SendPathWatch | null = null;

/** Register the app's idle watch so streaming turns can hold it active. */
export function registerIdleWatch(w: SendPathWatch | null): void {
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

/**
 * Note where the athlete is as a turn starts, and return the question to ask
 * if it fails: were they out of the app at any point while it ran?
 *
 * A turn that failed while nobody was looking — the idle stop dropped its
 * stream, or the platform dropped a backgrounded app's connection while the
 * athlete was authorizing Strava — may well have been answered, because the
 * server finishes a turn whether or not anyone is still reading it. One that
 * failed in front of the athlete was not lost to their absence, and is
 * reported as it stands. With no watch registered nobody can have been away.
 */
export function trackAbsence(): () => boolean {
  return watch?.trackAbsence() ?? (() => false);
}

/**
 * Run `work` once the athlete is here: now if they are, otherwise when they
 * come back to the app.
 *
 * With no watch registered there is nobody to wait for, and the work runs at
 * once.
 */
export function whenAthleteReturns(work: () => void): void {
  if (watch) {
    watch.whenPresent(work);
  } else {
    work();
  }
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
