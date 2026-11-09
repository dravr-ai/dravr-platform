// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Reads the provider cards, the imports and the onboarding status again when the import window finishes an import
// ABOUTME: The window is a page of its own, so its result reaches the provider list through the storage event it writes

import { useEffect } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { IMPORT_RESULT_STORAGE_KEY } from '@pierre/ui-logic';
import { QUERY_KEYS } from '../../constants/queryKeys';

/**
 * While the calling provider list is mounted, an import the window finishes
 * reloads what it changes: the cards (their history state), the imports
 * (the cards' imported counts) and the onboarding status (an import started
 * lets the connect step go on).
 */
export function useImportResultRefresh(): void {
  const queryClient = useQueryClient();
  useEffect(() => {
    const onStorage = (event: StorageEvent) => {
      if (event.key !== IMPORT_RESULT_STORAGE_KEY || !event.newValue) return;
      void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.providers.status() });
      void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.user.providerConnections() });
      void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.imports.all });
      void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.user.onboardingStatus() });
    };
    window.addEventListener('storage', onStorage);
    return () => window.removeEventListener('storage', onStorage);
  }, [queryClient]);
}
