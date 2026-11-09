// ABOUTME: The "Import your own data" link at the bottom of the provider list: one ink row, no card, opens the import window's source list
// ABOUTME: The way in for a source no provider card carries (Apple Health); the list it sits in refreshes through useImportResultRefresh
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useTranslation } from '@pierre/i18n';
import { openImportWindow } from './openImportWindow';

/** The row's look in the list it closes: the provider rows' hairline, or none in a section of its own. */
interface ImportOwnDataLinkProps {
  /** Draw the hairline the provider rows above it carry. */
  hairline?: boolean;
}

/**
 * Opens the import window on its source list (no hash), where every source
 * the server offers is listed — Apple Health among them, which has no
 * provider card and so no card's export link. The provider list it sits in
 * mounts `useImportResultRefresh`, which reads the cards and imports again
 * when the window finishes an import.
 */
export default function ImportOwnDataLink({ hairline = true }: ImportOwnDataLinkProps) {
  const { t } = useTranslation();
  return (
    <button
      type="button"
      onClick={() => openImportWindow()}
      data-testid="import-own-data-link"
      className={`group flex w-full items-center gap-3 py-3 text-left focus:outline-none focus-visible:ring-2 focus-visible:ring-primary/50 ${
        hairline ? 'border-t ghost-border-faint' : ''
      }`}
    >
      <span aria-hidden="true" className="flex h-6 w-6 flex-shrink-0 items-center justify-center text-on-surface">
        <svg className="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.75} d="M12 4v11m0 0l-4-4m4 4l4-4M5 19h14" />
        </svg>
      </span>
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="text-sm font-medium text-on-surface">
          {t('providers.import.link')} <span aria-hidden="true">→</span>
        </span>
        <span className="text-xs text-on-surface-variant">{t('providers.import.linkHint')}</span>
      </span>
    </button>
  );
}
