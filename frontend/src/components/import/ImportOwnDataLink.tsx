// ABOUTME: The "Import your own data" link at the bottom of the provider list: one ink row, no card, opens the import window
// ABOUTME: Shown in onboarding's connect step and in Settings' connections, under the providers it does not replace
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useEffect } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { useTranslation } from '@pierre/i18n';
import { IMPORT_RESULT_STORAGE_KEY } from '@pierre/ui-logic';
import { QUERY_KEYS } from '../../constants/queryKeys';
import { openImportWindow } from './openImportWindow';

/** The row's look in the list it closes: the provider rows' hairline, or none in a section of its own. */
interface ImportOwnDataLinkProps {
  /** Draw the hairline the provider rows above it carry. */
  hairline?: boolean;
}

export default function ImportOwnDataLink({ hairline = true }: ImportOwnDataLinkProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();

  // The import window writes its result when an import finishes; the list
  // this link sits in reads the providers and the import state again.
  useEffect(() => {
    const onStorage = (event: StorageEvent) => {
      if (event.key !== IMPORT_RESULT_STORAGE_KEY || !event.newValue) return;
      void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.providers.status() });
      void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.imports.all });
      void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.user.onboardingStatus() });
    };
    window.addEventListener('storage', onStorage);
    return () => window.removeEventListener('storage', onStorage);
  }, [queryClient]);
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
