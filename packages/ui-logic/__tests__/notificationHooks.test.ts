// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the shared notification hooks — each client's badge poll, and what reading a notification refreshes
// ABOUTME: A read notification must leave the feed and the badge, which a single `notifications` prefix never reached

import { describe, it, expect, vi } from 'vitest';
import { createElement, type ReactNode } from 'react';
import { renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider, type QueryObserverOptions } from '@tanstack/react-query';
import type { NotificationsApi } from '@pierre/api-client';
import { createNotificationHooks, type UnreadCountPolling } from '../src/notificationHooks';

const WEB_POLL: UnreadCountPolling = {
  staleTime: 60_000,
  refetchInterval: 120_000,
  refetchOnWindowFocus: false,
  refetchOnMount: false,
};
const MOBILE_POLL: UnreadCountPolling = { staleTime: 30_000, refetchInterval: 60_000 };

function setup(polling: UnreadCountPolling) {
  const api = {
    listNotifications: vi.fn().mockResolvedValue({ data: [{ id: 'n1' }], total: 1, unread_count: 1 }),
    getUnreadCount: vi.fn().mockResolvedValue({ unread_count: 4 }),
    markAsRead: vi.fn().mockResolvedValue(undefined),
    markAllAsRead: vi.fn().mockResolvedValue(undefined),
    deleteNotification: vi.fn().mockResolvedValue(undefined),
    getPreferences: vi.fn().mockResolvedValue({ preferences: [] }),
    updatePreference: vi.fn().mockResolvedValue(undefined),
  };
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client }, children);
  const hooks = createNotificationHooks(api as unknown as NotificationsApi, polling);
  return { api, client, wrapper, hooks };
}

function optionsOf(client: QueryClient, queryKey: readonly unknown[]) {
  return client.getQueryCache().find({ queryKey, exact: true })?.options as QueryObserverOptions | undefined;
}

describe('createNotificationHooks — the unread badge', () => {
  it('polls as the web client asks: 60 s fresh, every 120 s, no focus or mount refetch', async () => {
    const { client, wrapper, hooks } = setup(WEB_POLL);

    const { result } = renderHook(() => hooks.useUnreadCount(), { wrapper });

    await waitFor(() => expect(result.current.unreadCount).toBe(4));
    const options = optionsOf(client, ['notifications-unread-count']);
    expect(options?.staleTime).toBe(60_000);
    expect(options?.refetchInterval).toBe(120_000);
    expect(options?.refetchOnWindowFocus).toBe(false);
    expect(options?.refetchOnMount).toBe(false);
  });

  it('polls as the mobile client asks: 30 s fresh, every 60 s, the client defaults for focus and mount', async () => {
    const { client, wrapper, hooks } = setup(MOBILE_POLL);

    const { result } = renderHook(() => hooks.useUnreadCount(), { wrapper });

    await waitFor(() => expect(result.current.unreadCount).toBe(4));
    const options = optionsOf(client, ['notifications-unread-count']);
    expect(options?.staleTime).toBe(30_000);
    expect(options?.refetchInterval).toBe(60_000);
    expect(options?.refetchOnWindowFocus).toBeUndefined();
    expect(options?.refetchOnMount).toBeUndefined();
  });
});

describe('createNotificationHooks — the feed and its actions', () => {
  it('reads the feed for a category under that category key', async () => {
    const { api, client, wrapper, hooks } = setup(MOBILE_POLL);

    const { result } = renderHook(() => hooks.useNotificationFeed({ category: 'training' }), { wrapper });

    await waitFor(() => expect(result.current.notifications).toHaveLength(1));
    expect(api.listNotifications).toHaveBeenCalledWith({ category: 'training' });
    expect(result.current.unreadCount).toBe(1);
    expect(optionsOf(client, ['notifications-feed', 'training'])?.staleTime).toBe(30_000);
  });

  it('marking a notification read refreshes the category feed and the badge', async () => {
    const { api, client, wrapper, hooks } = setup(MOBILE_POLL);
    client.setQueryData(['notifications-feed', 'training'], { data: [{ id: 'n1' }], total: 1, unread_count: 1 });
    client.setQueryData(['notifications-unread-count'], { unread_count: 1 });
    client.setQueryData(['notifications-preferences'], { preferences: [] });

    const { result } = renderHook(() => hooks.useNotificationActions(), { wrapper });
    result.current.markAsRead('n1');

    await waitFor(() =>
      expect(client.getQueryState(['notifications-feed', 'training'])?.isInvalidated).toBe(true),
    );
    expect(api.markAsRead).toHaveBeenCalledWith('n1');
    expect(client.getQueryState(['notifications-unread-count'])?.isInvalidated).toBe(true);
    expect(client.getQueryState(['notifications-preferences'])?.isInvalidated).toBe(true);
  });

  it('forwards a preference change verbatim and refreshes the preferences', async () => {
    const { api, client, wrapper, hooks } = setup(WEB_POLL);
    const invalidate = vi.spyOn(client, 'invalidateQueries');

    const { result } = renderHook(() => hooks.useNotificationPreferences(), { wrapper });
    result.current.updatePreference({ category: 'training', enabled: false });

    await waitFor(() =>
      expect(invalidate).toHaveBeenCalledWith({ queryKey: ['notifications-preferences'] }),
    );
    expect(api.updatePreference).toHaveBeenCalledTimes(1);
    expect(api.updatePreference.mock.calls[0]).toEqual([{ category: 'training', enabled: false }]);
  });
});
