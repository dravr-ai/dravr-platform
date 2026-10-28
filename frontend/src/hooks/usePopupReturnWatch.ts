// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Notices an athlete back on our tab while a sign-in popup's promise never settles
// ABOUTME: Google's COOP header hides popup.closed from Firebase, so the page has to judge for itself

import { useCallback, useEffect, useRef } from 'react';

/**
 * How long the page may hold focus with no popup result before the attempt
 * counts as abandoned.
 *
 * Firebase's own popup operation waits 8 s (`_Timeout.AUTH_EVENT`) after it
 * sees the popup close before it rejects with `auth/popup-closed-by-user`,
 * because the sign-in network exchange can still be in flight. Matching it
 * means this watch never gives up sooner than the SDK would have, had it been
 * able to see the close — and a result that arrives later still signs the
 * athlete in, so the bound only decides how long the spinner can lie.
 */
export const POPUP_RETURN_GRACE_MS = 8000;

/** Stops a watch: removes its listeners and clears its timer. Safe to call twice. */
export type StopPopupReturnWatch = () => void;

/**
 * Watches for the athlete coming back to this tab while a popup flow is in
 * flight, and calls `onAbandoned` once if no result arrives within
 * {@link POPUP_RETURN_GRACE_MS} of that return.
 *
 * Why the page has to judge: Firebase detects a closed popup by polling
 * `popup.closed`, and accounts.google.com sets Cross-Origin-Opener-Policy, so
 * Chrome blocks that read ("Cross-Origin-Opener-Policy policy would block the
 * window.closed call"). When the result handoff also fails — third-party
 * storage partitioning with a cross-site authDomain — `signInWithPopup` never
 * settles, and a spinner waiting on it spins until reload.
 *
 * Focus (or the tab becoming visible) starts the grace timer; blur (or the tab
 * hiding) cancels it, since the athlete went back to the popup. Every watch
 * still running is stopped on unmount.
 */
export function usePopupReturnWatch(): (onAbandoned: () => void) => StopPopupReturnWatch {
  const active = useRef(new Set<StopPopupReturnWatch>());

  useEffect(() => {
    const watches = active.current;
    return () => {
      for (const stop of [...watches]) stop();
    };
  }, []);

  return useCallback((onAbandoned: () => void) => {
    let timer: ReturnType<typeof setTimeout> | null = null;

    const clearTimer = () => {
      if (timer !== null) {
        clearTimeout(timer);
        timer = null;
      }
    };

    const onReturn = () => {
      if (timer !== null) return;
      timer = setTimeout(() => {
        timer = null;
        stop();
        onAbandoned();
      }, POPUP_RETURN_GRACE_MS);
    };

    const onVisibility = () => {
      if (document.visibilityState === 'visible') onReturn();
      else clearTimer();
    };

    const stop: StopPopupReturnWatch = () => {
      clearTimer();
      window.removeEventListener('focus', onReturn);
      window.removeEventListener('blur', clearTimer);
      document.removeEventListener('visibilitychange', onVisibility);
      active.current.delete(stop);
    };

    window.addEventListener('focus', onReturn);
    window.addEventListener('blur', clearTimer);
    document.addEventListener('visibilitychange', onVisibility);
    active.current.add(stop);
    return stop;
  }, []);
}
