// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tracks whether the browser has a network connection, for the offline banner and error copy
// ABOUTME: The service worker makes the app launch offline, so something has to say so out loud

import { useEffect, useState } from 'react';

/**
 * Whether the browser believes it has a connection.
 *
 * The app precaches its shell, so an installed PWA opens offline and looks
 * completely healthy — the login form renders, accepts a password, and then
 * reports "sign-in failed" for what is really a dead radio. Every screen that
 * can fail on the network reads this so the failure can be named correctly.
 *
 * `navigator.onLine` is a floor, not a guarantee: it reports the link, not
 * reachability, so a captive portal still reads as online. It is right about
 * the case that matters here — radio off, airplane mode, tunnel — and being
 * wrong in the optimistic direction only costs the athlete the same generic
 * network error they get today.
 */
export function useOnlineStatus(): boolean {
  const [online, setOnline] = useState(() =>
    typeof navigator === 'undefined' ? true : navigator.onLine,
  );

  useEffect(() => {
    if (typeof window === 'undefined') return;
    const goOnline = () => setOnline(true);
    const goOffline = () => setOnline(false);
    window.addEventListener('online', goOnline);
    window.addEventListener('offline', goOffline);
    // The events only fire on a transition, so a tab that was already offline
    // when this mounted would never hear one. Read the current value now.
    setOnline(navigator.onLine);
    return () => {
      window.removeEventListener('online', goOnline);
      window.removeEventListener('offline', goOffline);
    };
  }, []);

  return online;
}
