// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests that the thread's connection notice names the provider its card shows, never the raw backend id
// ABOUTME: The id reaches the hook from the OAuth callback; the brand comes from the providers status the thread holds (carnet#1207)

import type { ReactNode } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { act, renderHook } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { QUERY_KEYS } from '../../../constants/queryKeys';
import { useOAuthCompletion } from '../useOAuthCompletion';

function renderCompletion(queryClient: QueryClient) {
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
  return renderHook(() => useOAuthCompletion(vi.fn()), { wrapper });
}

function completeOAuth(provider: string) {
  act(() => {
    window.dispatchEvent(
      new MessageEvent('message', {
        origin: window.location.origin,
        data: { type: 'oauth_completed', provider, success: true },
      }),
    );
  });
}

describe('useOAuthCompletion — the connection notice', () => {
  it("names the card's display name for the backend that connected", () => {
    const queryClient = new QueryClient();
    queryClient.setQueryData(QUERY_KEYS.providers.status(), {
      providers: [{ provider: 'intervals_icu', display_name: 'Intervals.icu' }],
    });
    const { result } = renderCompletion(queryClient);

    completeOAuth('intervals_icu');

    expect(result.current.notification?.provider).toBe('Intervals.icu');
  });

  it('falls back to the capitalised id before the status has answered', () => {
    const { result } = renderCompletion(new QueryClient());

    completeOAuth('whoop');

    expect(result.current.notification?.provider).toBe('Whoop');
  });
});
