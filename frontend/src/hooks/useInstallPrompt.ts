// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Captures beforeinstallprompt so the app can offer Add to Home Screen itself
// ABOUTME: Nothing listened for it, so installing meant finding a buried browser menu item

import { useCallback, useEffect, useState } from 'react';

/**
 * The event Chromium fires when it decides the app is installable.
 *
 * Not in lib.dom yet, so it is described here rather than cast away at the
 * call site.
 */
interface BeforeInstallPromptEvent extends Event {
  prompt: () => Promise<void>;
  userChoice: Promise<{ outcome: 'accepted' | 'dismissed' }>;
}

/** Remembers a dismissal so the nudge does not reappear every single session. */
const DISMISSED_KEY = 'dravr.installPromptDismissed';

/**
 * Whether the app is already running as an installed app.
 *
 * Both spellings matter: `display-mode: standalone` is the standard, and
 * `navigator.standalone` is the iOS-only one Safari still uses for a
 * home-screen launch.
 */
function isInstalled(): boolean {
  if (typeof window === 'undefined') return false;
  const standaloneDisplay = window.matchMedia?.('(display-mode: standalone)').matches ?? false;
  const iosStandalone = (window.navigator as { standalone?: boolean }).standalone === true;
  return standaloneDisplay || iosStandalone;
}

export interface UseInstallPromptReturn {
  /** True only when the browser offered an install AND the viewer has not declined. */
  canInstall: boolean;
  /** Show the browser's own install dialog. Resolves once the viewer answers. */
  promptInstall: () => Promise<void>;
  /** Hide the affordance for good on this device. */
  dismiss: () => void;
}

/**
 * Offer to install, on the platforms that let an app ask.
 *
 * Chromium fires `beforeinstallprompt` and lets the page defer it; nothing in
 * the app listened, so the event fired and was discarded on every load and the
 * only route to installing was the browser's own menu. Safari has no such
 * event — iOS installs are Share → Add to Home Screen and cannot be triggered
 * from script, so `canInstall` simply stays false there rather than the app
 * pretending otherwise.
 */
export function useInstallPrompt(): UseInstallPromptReturn {
  const [deferred, setDeferred] = useState<BeforeInstallPromptEvent | null>(null);
  const [dismissed, setDismissed] = useState(() => {
    try {
      return localStorage.getItem(DISMISSED_KEY) === 'true';
    } catch {
      // Private mode or blocked storage: treat as not dismissed rather than
      // suppressing the affordance for everyone whose browser refuses reads.
      return false;
    }
  });

  useEffect(() => {
    if (typeof window === 'undefined') return;

    const onBeforeInstall = (event: Event) => {
      // Preventing the default is what keeps the event usable later; without
      // it Chromium shows its own mini-infobar and discards the deferral.
      event.preventDefault();
      setDeferred(event as BeforeInstallPromptEvent);
    };
    const onInstalled = () => setDeferred(null);

    window.addEventListener('beforeinstallprompt', onBeforeInstall);
    window.addEventListener('appinstalled', onInstalled);
    return () => {
      window.removeEventListener('beforeinstallprompt', onBeforeInstall);
      window.removeEventListener('appinstalled', onInstalled);
    };
  }, []);

  const promptInstall = useCallback(async () => {
    if (!deferred) return;
    await deferred.prompt();
    const { outcome } = await deferred.userChoice;
    // The event is single-use whichever way it went.
    setDeferred(null);
    if (outcome === 'dismissed') {
      // Declining the OS dialog is a decision; do not ask again unprompted.
      setDismissed(true);
      try {
        localStorage.setItem(DISMISSED_KEY, 'true');
      } catch {
        // Nothing to do — the in-memory flag still hides it this session.
      }
    }
  }, [deferred]);

  const dismiss = useCallback(() => {
    setDismissed(true);
    try {
      localStorage.setItem(DISMISSED_KEY, 'true');
    } catch {
      // As above.
    }
  }, []);

  return {
    canInstall: deferred !== null && !dismissed && !isInstalled(),
    promptInstall,
    dismiss,
  };
}
