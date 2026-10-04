// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Takes a waiting build by itself, and asks first only when typed text would be lost
// ABOUTME: autoUpdate reloaded the tab under whoever was typing, losing the unsent message

import { useCallback, useEffect, useRef, useState } from 'react';
import { useRegisterSW } from 'virtual:pwa-register/react';
import { useTranslation } from '@pierre/i18n';
import { useAuth } from '../hooks/useAuth';
import { typedTextWouldBeLost } from '../utils/typedText';

/**
 * The "a new version is ready" strip, and the rule for when it is needed.
 *
 * The service worker used to register with `autoUpdate`, which activates a new
 * worker and reloads the page as soon as one is available. On a chat surface
 * that is destructive: the reload can land while an athlete is part-way
 * through a message, and the draft is not persisted anywhere.
 *
 * So the worker installs and waits, and what happens next depends on what a
 * reload would cost. With no typed text on the page it costs nothing, and the
 * build is taken without a question — that is every sign-in, where the worker
 * is found as the page loads and is ready seconds after the athlete arrives.
 * Typed text means a field the athlete edited and has not emptied again; a
 * value the page filled in itself, such as the saved display name on the
 * settings form, is not a draft and does not ask.
 * With typed text in a signed-in session this asks, and dismissing keeps the
 * old build until the next natural reload. On the sign-in form it neither
 * asks nor reloads over the credentials being typed; the build is taken when
 * the session opens and the form is gone.
 *
 * A waiting build is shared by every open tab, and the moment one tab takes it
 * the new worker controls them all. A tab holding typed text does not reload
 * under that either: it keeps asking, and its reload button reloads the page
 * rather than messaging a worker that is no longer waiting.
 */
export default function ServiceWorkerUpdatePrompt() {
  const { t } = useTranslation();
  const { isAuthenticated } = useAuth();
  const [asking, setAsking] = useState(false);
  // The new worker already controls this page because another tab took the
  // build, and the reload was held back for the typed text here.
  const [activatedElsewhere, setActivatedElsewhere] = useState(false);
  // This tab asked for the build itself, so the reload that follows is wanted
  // whatever is typed: either nothing was, or the athlete pressed reload.
  const takenHere = useRef(false);
  const {
    needRefresh: [needRefresh, setNeedRefresh],
    updateServiceWorker,
  } = useRegisterSW({
    onNeedReload() {
      if (takenHere.current || !typedTextWouldBeLost()) {
        window.location.reload();
        return;
      }
      setActivatedElsewhere(true);
    },
    onRegisterError(error) {
      console.error('Service worker registration failed:', error);
    },
  });

  const take = useCallback(() => {
    takenHere.current = true;
    if (activatedElsewhere) {
      window.location.reload();
      return;
    }
    void updateServiceWorker(true);
  }, [activatedElsewhere, updateServiceWorker]);

  useEffect(() => {
    if (!needRefresh) {
      setAsking(false);
      return;
    }
    if (typedTextWouldBeLost()) {
      setAsking(isAuthenticated);
      return;
    }
    take();
  }, [needRefresh, isAuthenticated, take]);

  if (!needRefresh || !asking) {
    return null;
  }

  return (
    <div
      role="status"
      aria-live="polite"
      data-testid="sw-update-prompt"
      className="fixed left-1/2 -translate-x-1/2 z-50 w-[min(28rem,calc(100vw-2rem))]"
      style={{ bottom: 'calc(1rem + env(safe-area-inset-bottom, 0px))' }}
    >
      <div className="flex items-center gap-3 rounded-xl border ghost-border bg-surface-container-low/95 px-4 py-3 backdrop-blur-sm">
        <p className="flex-1 text-sm text-on-surface">{t('shell.updateReady')}</p>
        <button
          type="button"
          onClick={() => setNeedRefresh(false)}
          className="touch-target rounded-lg px-3 text-sm text-on-surface-variant transition-colors hover:text-on-surface focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
        >
          {t('shell.updateLater')}
        </button>
        <button
          type="button"
          onClick={take}
          className="touch-target rounded-lg bg-primary px-4 text-sm font-medium text-on-primary transition-colors hover:opacity-90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2"
        >
          {t('shell.updateReload')}
        </button>
      </div>
    </div>
  );
}
