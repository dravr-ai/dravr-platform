// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query hooks for notification management on the web frontend
// ABOUTME: Provides hooks for notification feed, unread count, and mutation actions

import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { useCallback } from 'react';
import { QUERY_KEYS } from '@pierre/shared-constants';
import { notificationsApi } from '../services/api';
import type { ListNotificationsParams } from '@pierre/shared-types';

/**
 * Hook for fetching the notification feed with pagination and filtering.
 */
export function useNotificationFeed(params?: ListNotificationsParams) {
  const queryClient = useQueryClient();

  const query = useQuery({
    queryKey: QUERY_KEYS.notifications.feed(params?.category),
    queryFn: () => notificationsApi.listNotifications(params),
    staleTime: 30_000,
  });

  const invalidate = useCallback(async () => {
    await queryClient.invalidateQueries({
      queryKey: QUERY_KEYS.notifications.all,
    });
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

/**
 * Hook for fetching the unread notification count.
 * Polls every 60 seconds for badge updates.
 */
export function useUnreadCount() {
  const query = useQuery({
    queryKey: QUERY_KEYS.notifications.unreadCount(),
    queryFn: () => notificationsApi.getUnreadCount(),
    staleTime: 30_000,
    refetchInterval: 60_000,
  });

  return {
    unreadCount: query.data?.unread_count ?? 0,
    isLoading: query.isLoading,
  };
}

/**
 * Hook for notification mutations (mark read, mark all read, delete).
 */
export function useNotificationActions() {
  const queryClient = useQueryClient();

  const invalidateAll = useCallback(async () => {
    await queryClient.invalidateQueries({
      queryKey: QUERY_KEYS.notifications.all,
    });
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
