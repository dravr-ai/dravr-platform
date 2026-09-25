// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the web client's binding of the shared @pierre/ui-logic hooks — its API instance and its freshness
// ABOUTME: The hooks' behaviour is tested once in ui-logic; what is web's own is tested here

import { describe, it, expect, vi } from 'vitest';
import { renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider, type QueryObserverOptions } from '@tanstack/react-query';
import type { ReactNode } from 'react';

const { groupsApi, notificationsApi, featureFlagsApi, chatApi } = vi.hoisted(() => ({
  groupsApi: {
    getGroup: vi.fn().mockResolvedValue({ id: 'g1', name: 'Tempo Crew' }),
    listMembers: vi.fn().mockResolvedValue({ members: [] }),
    getStats: vi.fn().mockResolvedValue({ stats: null }),
  },
  notificationsApi: { getUnreadCount: vi.fn().mockResolvedValue({ unread_count: 3 }) },
  featureFlagsApi: {
    getMyFeatures: vi.fn().mockResolvedValue({ flags: { billing_header: true }, known: [] }),
  },
  chatApi: { listCommands: vi.fn().mockResolvedValue([]) },
}));

vi.mock('../../services/api', () => ({ groupsApi, notificationsApi, featureFlagsApi, chatApi }));

import { useGroup, useGroupMembers, useGroupStats } from '../useGroups';
import { useUnreadCount } from '../useNotifications';
import { useFeatureFlags, FEATURE_KEYS } from '../useFeatureFlags';
import { useCommandPalette } from '../useCommandPalette';

function setup() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  const optionsOf = (queryKey: readonly unknown[]) =>
    client.getQueryCache().find({ queryKey, exact: true })?.options as QueryObserverOptions | undefined;
  return { wrapper, optionsOf };
}

describe('web bindings of the shared hooks', () => {
  it('keeps the web group freshness: group and members 30 s, stats 60 s', async () => {
    const { wrapper, optionsOf } = setup();

    const { result } = renderHook(
      () => ({ group: useGroup('g1'), members: useGroupMembers('g1'), stats: useGroupStats('g1') }),
      { wrapper },
    );

    await waitFor(() => expect(result.current.group.group).toEqual({ id: 'g1', name: 'Tempo Crew' }));
    expect(groupsApi.getGroup).toHaveBeenCalledWith('g1');
    expect(optionsOf(['groups', 'g1'])?.staleTime).toBe(30_000);
    expect(optionsOf(['groups', 'g1', 'members'])?.staleTime).toBe(30_000);
    expect(optionsOf(['groups', 'g1', 'stats'])?.staleTime).toBe(60_000);
  });

  it('polls the unread badge every two minutes and not on focus or mount', async () => {
    const { wrapper, optionsOf } = setup();

    const { result } = renderHook(() => useUnreadCount(), { wrapper });

    await waitFor(() => expect(result.current.unreadCount).toBe(3));
    const options = optionsOf(['notifications-unread-count']);
    expect(options?.staleTime).toBe(60_000);
    expect(options?.refetchInterval).toBe(120_000);
    expect(options?.refetchOnWindowFocus).toBe(false);
    expect(options?.refetchOnMount).toBe(false);
  });

  it('reads the flags through the web feature flags API', async () => {
    const { wrapper } = setup();

    const { result } = renderHook(() => useFeatureFlags(), { wrapper });

    await waitFor(() => expect(result.current.flags[FEATURE_KEYS.billingHeader]).toBe(true));
    expect(result.current.flags[FEATURE_KEYS.apiTokens]).toBe(false);
  });

  it('asks the web chat API for the conversation the composer passes', async () => {
    const { wrapper } = setup();

    renderHook(() => useCommandPalette({ value: '/', conversationId: 'conv-7', onChange: vi.fn() }), {
      wrapper,
    });

    await waitFor(() => expect(chatApi.listCommands).toHaveBeenCalledWith('conv-7'));
  });
});
