// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The empty thread's starters hook hands back starters only while the new thread is the one shown
// ABOUTME: Red if a disabled read leaks the starters an earlier read cached into an open thread, a coach's plan thread among them

import React from 'react';
import { renderHook } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { UseCaseStarter } from '@pierre/shared-types';

const mockListStarters = jest.fn<Promise<UseCaseStarter[]>, []>();
jest.mock('../src/services/api', () => ({
  chatApi: { listStarters: () => mockListStarters() },
}));

import { useEmptyThreadStarters } from '../src/hooks/useEmptyThreadStarters';

const CACHED: UseCaseStarter[] = [{ id: 'meet_team', label: 'Meet my team', postback: 'uc:0:meet_team' }];

function wrapperWith(client: QueryClient) {
  return function Wrapper({ children }: { children: React.ReactNode }) {
    return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
  };
}

describe('useEmptyThreadStarters', () => {
  it('returns nothing while the thread shown is not the new one, even with starters cached from an earlier read', () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    client.setQueryData(QUERY_KEYS.chat.starters(), CACHED);
    const { result } = renderHook(() => useEmptyThreadStarters(false), { wrapper: wrapperWith(client) });
    expect(result.current).toEqual([]);
    expect(mockListStarters).not.toHaveBeenCalled();
  });

  it('returns the cached starters while the new thread is shown', () => {
    mockListStarters.mockResolvedValue(CACHED);
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    client.setQueryData(QUERY_KEYS.chat.starters(), CACHED);
    const { result } = renderHook(() => useEmptyThreadStarters(true), { wrapper: wrapperWith(client) });
    expect(result.current).toEqual(CACHED);
  });
});
