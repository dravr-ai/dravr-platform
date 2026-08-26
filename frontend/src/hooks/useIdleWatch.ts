// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Binds the browser's interaction and visibility signals to the shared idle contract
// ABOUTME: An idle tab stops polling and drops its open turn stream; the next interaction resumes it

import { useEffect } from 'react';
import { focusManager } from '@tanstack/react-query';
import { IdleWatch } from '@pierre/shared-constants';
import { idleAbort, registerIdleWatch, resetIdleAbort } from '../services/api/idleSignal';

/**
 * The interactions that count as "somebody is here".
 *
 * Deliberately includes `scroll` and `pointermove`: an athlete reading a long
 * reply produces neither clicks nor keystrokes, and cutting them off would be
 * the failure mode that makes an idle stop feel broken. `visibilitychange` is
 * handled separately because it is a state, not an interaction.
 *
 * All are registered passive and capturing, so they observe without competing
 * with any handler and without blocking scroll.
 */
const INTERACTION_EVENTS = [
  'pointerdown',
  'pointermove',
  'keydown',
  'wheel',
  'scroll',
  'touchstart',
] as const;

/**
 * Stop talking to the server when nobody is driving the tab.
 *
 * React Query already pauses interval refetches while the document is hidden.
 * What it cannot know is that a *visible* tab has been sitting untouched on a
 * second monitor for an hour — and that is the case that keeps a Cloud Run
 * instance warm and billed. This hook closes it: after
 * `IDLE_STOP_AFTER_MS` with no interaction the tab is marked unfocused, which
 * stops every recurring poll, and any turn still streaming is aborted. The
 * next pointer move, key, or scroll resumes both.
 *
 * `focusManager.setFocused` is the single switch, so the idle stop and the
 * hidden-tab pause are the same mechanism rather than two that must agree.
 * Taking manual ownership of it means this hook also owns the visibility
 * half — once a boolean is set explicitly, React Query stops consulting
 * `document.visibilityState` on its own.
 *
 * Mounted once, at the application root.
 */
export function useIdleWatch(): void {
  useEffect(() => {
    const watch = new IdleWatch({
      onIdle: () => {
        focusManager.setFocused(false);
        // A turn still streaming holds the connection — and the instance
        // behind it — open indefinitely. The athlete re-sends on their way
        // back in; `sendTurn` tells them so in as many words.
        idleAbort();
      },
      onActive: () => {
        resetIdleAbort();
        focusManager.setFocused(true);
      },
    });

    registerIdleWatch(watch);

    const noteInteraction = () => watch.noteInteraction();
    for (const event of INTERACTION_EVENTS) {
      window.addEventListener(event, noteInteraction, { passive: true, capture: true });
    }

    // A hidden tab is idle immediately — there is no threshold to wait out
    // when the athlete has demonstrably looked away.
    const onVisibilityChange = () => {
      if (document.visibilityState === 'visible') {
        watch.noteInteraction();
      } else {
        watch.suspend();
      }
    };
    document.addEventListener('visibilitychange', onVisibilityChange);

    return () => {
      registerIdleWatch(null);
      for (const event of INTERACTION_EVENTS) {
        window.removeEventListener(event, noteInteraction, { capture: true });
      }
      document.removeEventListener('visibilitychange', onVisibilityChange);
      watch.stop();
      // Hand focus back to React Query's own visibility tracking so a
      // remount (or a test) does not inherit a pinned `false`.
      focusManager.setFocused(undefined);
    };
  }, []);
}
