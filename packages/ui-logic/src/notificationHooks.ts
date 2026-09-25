// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query hooks for the notification feed, unread badge, actions and per-category preferences
// ABOUTME: Bound to each client's notifications API; only the unread-badge poll differs between the clients

import { useCallback } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { NotificationsApi } from '@pierre/api-client';
import type {
  ListNotificationsParams,
  UpdateNotificationPreferenceRequest,
} from '@pierre/shared-types';

/**
 * How a client polls the unread-notification badge.
 *
 * Web reads it fresh for 60 s, polls every 120 s, and skips the refetch on
 * window focus and on mount, because tab-hopping generated 80+ requests per
 * session for a low-value background counter. Mobile reads it fresh for 30 s,
 * polls every 60 s, and keeps its client defaults for focus and mount. Each
 * client passes its own values rather than one being chosen for both.
 */
export interface UnreadCountPolling {
  staleTime: number;
  refetchInterval: number;
  refetchOnWindowFocus?: boolean;
  refetchOnMount?: boolean;
}

/** Notification reads both clients keep fresh for the same span. */
const FEED_STALE_MS = 30_000;
const PREFERENCES_STALE_MS = 5 * 60_000;

/**
 * The feed's key prefix, every category at once. `QUERY_KEYS.notifications.feed()`
 * names one category (`undefined` included), which would match only itself.
 */
const FEED_PREFIX = ['notifications-feed'] as const;

/**
 * Build the notification hooks over one client's notifications API.
 *
 * Each client binds this once, in its own `hooks/useNotifications`, with its
 * API instance and its {@link UnreadCountPolling}.
 */
export function createNotificationHooks(
  notificationsApi: NotificationsApi,
  unreadCountPolling: UnreadCountPolling,
) {
  /** The notification feed, paginated and filtered by `params`. */
  function useNotificationFeed(params?: ListNotificationsParams) {
    const queryClient = useQueryClient();

    const query = useQuery({
      queryKey: QUERY_KEYS.notifications.feed(params?.category),
      queryFn: () => notificationsApi.listNotifications(params),
      staleTime: FEED_STALE_MS,
    });

    const invalidate = useCallback(async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: QUERY_KEYS.notifications.all }),
        queryClient.invalidateQueries({ queryKey: FEED_PREFIX }),
        queryClient.invalidateQueries({ queryKey: QUERY_KEYS.notifications.unreadCount() }),
      ]);
    }, [queryClient]);

    return {
      notifications: query.data?.data ?? [],
      total: query.data?.total ?? 0,
      unreadCount: query.data?.unread_count ?? 0,
      isLoading: query.isLoading,
      isRefetching: query.isRefetching,
      isError: query.isError,
      error: query.error,
      refetch: query.refetch,
      invalidate,
    };
  }

  /** The unread count behind the badge, polled as the client's {@link UnreadCountPolling} says. */
  function useUnreadCount() {
    const query = useQuery({
      queryKey: QUERY_KEYS.notifications.unreadCount(),
      queryFn: () => notificationsApi.getUnreadCount(),
      ...unreadCountPolling,
    });

    return {
      unreadCount: query.data?.unread_count ?? 0,
      isLoading: query.isLoading,
    };
  }

  /** Mark read, mark all read, delete. */
  function useNotificationActions() {
    const queryClient = useQueryClient();

    // Every distinct notifications key. A single prefix match cannot reach
    // them: the key tree is flat and uses dashed sentinels
    // (`notifications-feed`, `notifications-unread-count`) rather than nested
    // children of `['notifications']`, so invalidating `all` alone leaves the
    // feed and the badge showing a notification the athlete just read.
    const invalidateAll = useCallback(async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: QUERY_KEYS.notifications.all }),
        queryClient.invalidateQueries({ queryKey: FEED_PREFIX }),
        queryClient.invalidateQueries({ queryKey: QUERY_KEYS.notifications.unreadCount() }),
        queryClient.invalidateQueries({ queryKey: QUERY_KEYS.notifications.preferences() }),
        queryClient.invalidateQueries({ queryKey: QUERY_KEYS.notifications.devices() }),
      ]);
    }, [queryClient]);

    const markAsRead = useMutation({
      mutationFn: (notificationId: string) => notificationsApi.markAsRead(notificationId),
      onSuccess: invalidateAll,
    });

    const markAllAsRead = useMutation({
      mutationFn: () => notificationsApi.markAllAsRead(),
      onSuccess: invalidateAll,
    });

    const deleteNotification = useMutation({
      mutationFn: (notificationId: string) => notificationsApi.deleteNotification(notificationId),
      onSuccess: invalidateAll,
    });

    return {
      markAsRead: markAsRead.mutate,
      markAllAsRead: markAllAsRead.mutate,
      deleteNotification: deleteNotification.mutate,
      isMarkingRead: markAsRead.isPending,
      isMarkingAllRead: markAllAsRead.isPending,
      isDeleting: deleteNotification.isPending,
    };
  }

  /**
   * Per-category notification preferences.
   *
   * `preferences` is the server's per-category list and `updatePreference`
   * takes an `UpdateNotificationPreferenceRequest` verbatim. Nothing is derived
   * locally — a category the server does not return is a category the client
   * does not offer.
   */
  function useNotificationPreferences() {
    const queryClient = useQueryClient();

    const query = useQuery({
      queryKey: QUERY_KEYS.notifications.preferences(),
      queryFn: () => notificationsApi.getPreferences(),
      staleTime: PREFERENCES_STALE_MS,
    });

    const updatePreference = useMutation({
      // Wrapped rather than passed by reference: React Query hands a mutation
      // context as a second argument, and forwarding that into the api-client
      // method would put an argument on the wire the method never declared.
      mutationFn: (request: UpdateNotificationPreferenceRequest) =>
        notificationsApi.updatePreference(request),
      onSuccess: () => {
        void queryClient.invalidateQueries({
          queryKey: QUERY_KEYS.notifications.preferences(),
        });
      },
    });

    return {
      preferences: query.data?.preferences ?? [],
      isLoading: query.isLoading,
      isError: query.isError,
      updatePreference: updatePreference.mutate,
      isUpdating: updatePreference.isPending,
    };
  }

  return {
    useNotificationFeed,
    useUnreadCount,
    useNotificationActions,
    useNotificationPreferences,
  };
}
