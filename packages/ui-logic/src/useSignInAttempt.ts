// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Names one opening of a sign-in window with an attempt id, and cancels that attempt on close
// ABOUTME: Closing the window mid-login asks the server to stop it, since a dropped connection never reaches it

import { useCallback, useEffect, useMemo, useRef } from 'react';

/**
 * The sign-in window phases in which the server holds the attempt: a step
 * in flight (`logging-in`, `verifying`, a phone approval being polled), a
 * flow parked on a 2FA pick or a code, or a browser parked at a provider's
 * robot check that the athlete is answering (`takeover`, ADR-034).
 */
const LIVE_PHASES: ReadonlySet<string> = new Set([
  'logging-in',
  'verifying',
  'two-factor',
  'waiting-approval',
  'number-match',
  'otp',
  'takeover',
]);

/** Whether closing the window in `phase` leaves a login running on the server. */
export function isSignInPhaseLive(phase: string): boolean {
  return LIVE_PHASES.has(phase);
}

/** What a sign-in window tells {@link useSignInAttempt}. */
export interface SignInAttemptOptions {
  /**
   * The window has a step in flight, or the server holds its flow parked
   * (a 2FA pick, a code, a phone approval): closing it now leaves a login
   * running on the server.
   */
  active: boolean;
  /** A fresh attempt id (a UUID v4), from the platform's own randomness. */
  mintAttemptId: () => string;
  /** Asks the server to cancel the attempt. Best effort: its failure is swallowed. */
  cancel: (attemptId: string) => Promise<unknown>;
}

/**
 * The attempt id of a window's current opening, and the way to start a new
 * attempt in the same opening.
 */
export interface SignInAttempt {
  /** The current attempt id; `undefined` between a close and the next opening. */
  (): string | undefined;
  /**
   * Start a new attempt while the window stays open: the server ended the
   * last one itself (a takeover that closed cancels its attempt there), and
   * every step naming it from then on is refused. Nothing is cancelled.
   */
  renew: () => void;
}

/**
 * The attempt id of the window's current opening, cancelled when it closes.
 *
 * Every opening (`open` turning true) mints a fresh id, which each step of
 * that window sends, so the server can tie the steps to one attempt. Closing
 * the window or unmounting it while `active` sends one cancel naming that id
 * and forgets it; a window closed with nothing running sends none. Between a
 * close and the next opening the getter returns `undefined`.
 *
 * The cancel is the only thing that reaches the server: an aborted request
 * drops its connection at the browser, and the hops in front of the API do
 * not carry that disconnect any further.
 */
export function useSignInAttempt(
  open: boolean,
  options: SignInAttemptOptions,
): SignInAttempt {
  const attemptId = useRef<string | null>(null);
  const latest = useRef(options);

  // Declared first, so the close below reads this render's `active`.
  useEffect(() => {
    latest.current = options;
  });

  const endAttempt = useCallback(() => {
    const id = attemptId.current;
    attemptId.current = null;
    if (id === null || !latest.current.active) return;
    latest.current.cancel(id).catch(() => undefined);
  }, []);

  useEffect(() => {
    if (open) {
      attemptId.current = latest.current.mintAttemptId();
      return;
    }
    endAttempt();
  }, [open, endAttempt]);

  useEffect(() => endAttempt, [endAttempt]);

  const current = useCallback(() => attemptId.current ?? undefined, []);
  const renew = useCallback(() => {
    if (attemptId.current !== null) attemptId.current = latest.current.mintAttemptId();
  }, []);
  return useMemo(() => Object.assign(current, { renew }), [current, renew]);
}
