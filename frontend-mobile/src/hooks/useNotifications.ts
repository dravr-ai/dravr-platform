// ABOUTME: React Query hooks for push notification management
// ABOUTME: Provides hooks for notification feed, unread count, preferences, and device tokens

import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { useCallback } from 'react';
import { QUERY_KEYS } from '../../../packages/shared-constants/src/query-keys';
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
    staleTime: 30_000, // 30 seconds
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
 * Polls every 60 seconds for real-time badge updates.
 */
export function useUnreadCount() {
  const query = useQuery({
    queryKey: QUERY_KEYS.notifications.unreadCount(),
    queryFn: () => notificationsApi.getUnreadCount(),
    staleTime: 30_000,
    refetchInterval: 60_000, // Poll every 60 seconds
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

/**
 * Hook for managing notification preferences.
 */
export function useNotificationPreferences() {
  const queryClient = useQueryClient();

  const query = useQuery({
    queryKey: QUERY_KEYS.notifications.preferences(),
    queryFn: () => notificationsApi.getPreferences(),
    staleTime: 5 * 60_000, // 5 minutes
  });

  const updatePreference = useMutation({
    mutationFn: notificationsApi.updatePreference,
    onSuccess: () => {
      queryClient.invalidateQueries({
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
