// ABOUTME: An import the import window finishes reloads the provider cards, the imports and the onboarding status
// ABOUTME: Pins the storage key it listens to, and that any other storage write reloads nothing
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi } from 'vitest';
import type { ReactNode } from 'react';
import { renderHook, act } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { IMPORT_RESULT_STORAGE_KEY } from '@pierre/ui-logic';
import { QUERY_KEYS } from '../../../constants/queryKeys';
import { useImportResultRefresh } from '../useImportResultRefresh';

function mount() {
  const client = new QueryClient();
  const invalidate = vi.spyOn(client, 'invalidateQueries').mockResolvedValue();
  const wrapper = ({ children }: { children: ReactNode }) => <QueryClientProvider client={client}>{children}</QueryClientProvider>;
  const hook = renderHook(() => useImportResultRefresh(), { wrapper });
  return { invalidate, hook };
}

const write = (key: string, newValue: string | null) =>
  act(() => {
    window.dispatchEvent(new StorageEvent('storage', { key, newValue }));
  });

describe('useImportResultRefresh', () => {
  it('reloads the cards, the imports and the onboarding status when an import finishes', () => {
    const { invalidate } = mount();

    write(IMPORT_RESULT_STORAGE_KEY, '{"source":"garmin_archive"}');

    const keys = invalidate.mock.calls.map(([filters]) => filters?.queryKey);
    expect(keys).toEqual(
      expect.arrayContaining([
        QUERY_KEYS.providers.status(),
        QUERY_KEYS.user.providerConnections(),
        QUERY_KEYS.imports.all,
        QUERY_KEYS.user.onboardingStatus(),
      ]),
    );
  });

  it('reloads nothing for another key, a cleared entry, or once unmounted', () => {
    const { invalidate, hook } = mount();

    write('pierre_oauth_result', '{}');
    write(IMPORT_RESULT_STORAGE_KEY, null);
    hook.unmount();
    write(IMPORT_RESULT_STORAGE_KEY, '{"source":"strava_archive"}');

    expect(invalidate).not.toHaveBeenCalled();
  });
});
