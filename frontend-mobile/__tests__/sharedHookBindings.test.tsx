// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the mobile client's binding of the shared @pierre/ui-logic hooks — its API instance, freshness and route
// ABOUTME: The hooks' behaviour is tested once in ui-logic; what is mobile's own is tested here

import React, { type ReactNode } from 'react';
import { renderHook, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider, type QueryObserverOptions } from '@tanstack/react-query';

const mockGetGroup = jest.fn();
const mockListMembers = jest.fn();
const mockGetStats = jest.fn();
const mockGetUnreadCount = jest.fn();
const mockGetMyFeatures = jest.fn();
const mockListCommands = jest.fn();
let mockRouteParams: { conversationId?: string } = {};

jest.mock('expo-router', () => ({
  useLocalSearchParams: () => mockRouteParams,
}));
jest.mock('../src/services/api', () => ({
  groupsApi: {
    getGroup: (id: string) => mockGetGroup(id),
    listMembers: (id: string) => mockListMembers(id),
    getStats: (id: string) => mockGetStats(id),
  },
  notificationsApi: { getUnreadCount: () => mockGetUnreadCount() },
  featureFlagsApi: { getMyFeatures: () => mockGetMyFeatures() },
  chatApi: { listCommands: (id?: string) => mockListCommands(id) },
}));

import { useGroup, useGroupMembers, useGroupStats } from '../src/hooks/useGroups';
import { useUnreadCount } from '../src/hooks/useNotifications';
import { useFeatureFlags, FEATURE_KEYS } from '../src/hooks/useFeatureFlags';
import { useCommandPalette } from '../src/hooks/useCommandPalette';

function setup() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  const optionsOf = (queryKey: readonly unknown[]) =>
    client.getQueryCache().find({ queryKey, exact: true })?.options as QueryObserverOptions | undefined;
  return { wrapper, optionsOf };
}

describe('mobile bindings of the shared hooks', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockRouteParams = {};
    mockGetGroup.mockResolvedValue({ id: 'g1', name: 'Tempo Crew' });
    mockListMembers.mockResolvedValue({ members: [] });
    mockGetStats.mockResolvedValue({ stats: null });
    mockGetUnreadCount.mockResolvedValue({ unread_count: 2 });
    mockListCommands.mockResolvedValue([]);
  });

  it('keeps the mobile group freshness: group and members 60 s, stats 5 min', async () => {
    const { wrapper, optionsOf } = setup();

    const { result } = renderHook(
      () => ({ group: useGroup('g1'), members: useGroupMembers('g1'), stats: useGroupStats('g1') }),
      { wrapper },
    );

    await waitFor(() => expect(result.current.group.group).toEqual({ id: 'g1', name: 'Tempo Crew' }));
    expect(mockGetGroup).toHaveBeenCalledWith('g1');
    expect(optionsOf(['groups', 'g1'])?.staleTime).toBe(60_000);
    expect(optionsOf(['groups', 'g1', 'members'])?.staleTime).toBe(60_000);
    expect(optionsOf(['groups', 'g1', 'stats'])?.staleTime).toBe(300_000);
  });

  it('polls the unread badge every minute', async () => {
    const { wrapper, optionsOf } = setup();

    const { result } = renderHook(() => useUnreadCount(), { wrapper });

    await waitFor(() => expect(result.current.unreadCount).toBe(2));
    const options = optionsOf(['notifications-unread-count']);
    expect(options?.staleTime).toBe(30_000);
    expect(options?.refetchInterval).toBe(60_000);
  });

  it('reads the flags through the mobile feature flags API', async () => {
    mockGetMyFeatures.mockResolvedValue({ flags: { api_tokens: true }, known: [] });
    const { wrapper } = setup();

    const { result } = renderHook(() => useFeatureFlags(), { wrapper });

    await waitFor(() => expect(result.current.flags[FEATURE_KEYS.apiTokens]).toBe(true));
    expect(result.current.flags[FEATURE_KEYS.billingHeader]).toBe(false);
  });

  it('answers the command palette for the routed conversation', async () => {
    mockRouteParams = { conversationId: 'conv-3' };
    const { wrapper } = setup();

    renderHook(() => useCommandPalette({ value: '/', onChange: jest.fn() }), { wrapper });

    await waitFor(() => expect(mockListCommands).toHaveBeenCalledWith('conv-3'));
  });

  it('reads the new-conversation sentinel as no conversation at all', async () => {
    mockRouteParams = { conversationId: 'new' };
    const { wrapper } = setup();

    renderHook(() => useCommandPalette({ value: '/', onChange: jest.fn() }), { wrapper });

    await waitFor(() => expect(mockListCommands).toHaveBeenCalledTimes(1));
    expect(mockListCommands).toHaveBeenCalledWith(undefined);
  });
});
