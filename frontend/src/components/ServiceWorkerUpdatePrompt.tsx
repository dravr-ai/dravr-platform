// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Offers the reload when a new build is waiting, instead of taking it unannounced
// ABOUTME: autoUpdate reloaded the tab under whoever was typing, losing the unsent message

import { useRegisterSW } from 'virtual:pwa-register/react';
import { useTranslation } from '@pierre/i18n';

/**
 * The "a new version is ready" strip.
 *
 * The service worker used to register with `autoUpdate`, which activates a new
 * worker and reloads the page as soon as one is available. On a dashboard that
 * is invisible and fine. On a chat surface it is destructive: the reload can
 * land while an athlete is part-way through a message, and the draft is not
 * persisted anywhere.
 *
 * So the worker installs and waits, and this asks. Dismissing keeps the old
 * build running until the next natural reload, which is the correct default —
 * nothing here is urgent enough to interrupt a sentence.
 */
export default function ServiceWorkerUpdatePrompt() {
  const { t } = useTranslation();
  const {
    needRefresh: [needRefresh, setNeedRefresh],
    updateServiceWorker,
  } = useRegisterSW({
    onRegisterError(error) {
      console.error('Service worker registration failed:', error);
    },
  });

  if (!needRefresh) {
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
          className="min-h-[44px] rounded-lg px-3 text-sm text-on-surface-variant transition-colors hover:text-on-surface focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
        >
          {t('shell.updateLater')}
        </button>
        <button
          type="button"
          onClick={() => void updateServiceWorker(true)}
          className="min-h-[44px] rounded-lg bg-primary px-4 text-sm font-medium text-on-primary transition-colors hover:opacity-90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2"
        >
          {t('shell.updateReload')}
        </button>
      </div>
    </div>
  );
}
