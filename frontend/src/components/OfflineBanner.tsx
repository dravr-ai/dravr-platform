// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Says out loud that the device has no connection, above every authenticated screen
// ABOUTME: Without it the precached shell opens offline looking healthy and blames the athlete

import { useTranslation } from '@pierre/i18n';
import { useOnlineStatus } from '../hooks/useOnlineStatus';

/**
 * The offline strip.
 *
 * `role="status"` with `aria-live="polite"` rather than `alert`: losing signal
 * is worth announcing but should not interrupt whatever a screen reader is
 * already reading. It renders nothing at all while online, so it costs a
 * healthy session no layout.
 *
 * `sticky top-0` matches ImpersonationBanner, and the two stack in the order
 * they are mounted rather than fighting for the same fixed slot. The top
 * safe-area padding matters on an installed iOS PWA: `viewport-fit=cover` plus
 * a translucent status bar means y=0 is under the notch.
 */
export default function OfflineBanner() {
  const { t } = useTranslation();
  const online = useOnlineStatus();

  if (online) {
    return null;
  }

  return (
    <div
      role="status"
      aria-live="polite"
      className="bg-error text-on-primary px-4 py-2 sticky top-0 z-50 shadow-lg"
      style={{ paddingTop: 'calc(0.5rem + env(safe-area-inset-top, 0px))' }}
      data-testid="offline-banner"
    >
      <div className="max-w-7xl mx-auto flex items-center gap-3">
        <svg
          className="w-5 h-5 flex-shrink-0"
          aria-hidden="true"
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M18.364 5.636L5.636 18.364M8.111 8.111A9 9 0 0112 7c2.485 0 4.735 1.007 6.364 2.636M12 20h.01"
          />
        </svg>
        <span className="text-sm font-medium">{t('shell.offlineBanner')}</span>
      </div>
    </div>
  );
}
