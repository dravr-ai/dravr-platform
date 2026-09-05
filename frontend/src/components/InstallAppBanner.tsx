// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Add to Home Screen offer, shown only where the browser says installing is possible
// ABOUTME: Renders nothing on iOS, where installing cannot be triggered from script at all

import { useTranslation } from '@pierre/i18n';
import { useInstallPrompt } from '../hooks/useInstallPrompt';

/**
 * A quiet install nudge.
 *
 * Deliberately not a modal and not on the login screen: it appears inside the
 * authenticated shell, for someone who has already decided to use the product.
 * `canInstall` is false unless the browser actually fired
 * `beforeinstallprompt`, so this never advertises an install the platform will
 * not honour — on iOS it renders nothing rather than explaining a Share-sheet
 * gesture nobody reads.
 */
export default function InstallAppBanner() {
  const { t } = useTranslation();
  const { canInstall, promptInstall, dismiss } = useInstallPrompt();

  if (!canInstall) {
    return null;
  }

  return (
    <div
      data-testid="install-banner"
      className="flex items-center gap-3 border-b ghost-border bg-surface-container-low/80 px-4 py-2"
    >
      <svg
        className="h-5 w-5 flex-shrink-0 text-primary"
        aria-hidden="true"
        fill="none"
        stroke="currentColor"
        viewBox="0 0 24 24"
      >
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M12 4v12m0 0l-4-4m4 4l4-4M4 20h16"
        />
      </svg>
      <p className="flex-1 text-sm text-on-surface">{t('shell.installPrompt')}</p>
      <button
        type="button"
        onClick={() => void promptInstall()}
        className="touch-target rounded-lg bg-primary px-4 text-sm font-medium text-on-primary transition-colors hover:opacity-90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2"
      >
        {t('shell.installAction')}
      </button>
      <button
        type="button"
        onClick={dismiss}
        aria-label={t('shell.installDismiss')}
        className="inline-flex h-11 w-11 items-center justify-center rounded-lg text-on-surface-variant transition-colors hover:text-on-surface focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
      >
        <svg className="h-4 w-4" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
        </svg>
      </button>
    </div>
  );
}
