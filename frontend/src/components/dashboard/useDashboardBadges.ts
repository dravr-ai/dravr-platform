// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Custom hooks for dashboard badge data (pending users, store stats, unread chats)
// ABOUTME: Enables sidebar badges to share query data with panel components

import { useQuery } from '@tanstack/react-query';
import { adminApi } from '../../services/api';
import { QUERY_KEYS } from '../../constants/queryKeys';
import type { ConversationScope } from '@pierre/chat-utils';
import { useUnreadConversationTotal } from '../../hooks/useConversationList';
import type { User } from '../../types/api';

/**
 * Hook to get pending users count for badge display.
 *
 * `enabled` should be set to the caller's admin check so non-admin sessions
 * don't fire `/api/admin/pending-users` and surface 403s in the console.
 */
export function usePendingUsersCount(enabled = true): number {
  const { data: pendingUsers = [] } = useQuery<User[]>({
    queryKey: QUERY_KEYS.adminUsers.pending(),
    queryFn: () => adminApi.getPendingUsers(),
    staleTime: 30_000,
    retry: false,
    enabled,
  });
  return pendingUsers.length;
}

/** Hook to get pending agent count for badge display */
export function useStoreStatsPendingCount(enabled = true): number {
  const { data: storeStats } = useQuery({
    queryKey: QUERY_KEYS.adminStore.stats(),
    queryFn: () => adminApi.getStoreStats(),
    staleTime: 30_000,
    retry: false,
    enabled,
  });
  return storeStats?.pending_count ?? 0;
}

/**
 * Unread rows across one side of the athlete's conversations: rooms for the
 * Groups nav badge, the athlete's own threads for Home's.
 *
 * Reads the same paged list query the lists draw, so opening a thread
 * (which zeroes its row) and the badge agree without a second request.
 * `enabled` is the caller's athlete check: an operator has neither tab.
 */
export function useUnreadConversationsCount(scope: ConversationScope, enabled = true): number {
  return useUnreadConversationTotal(scope, enabled);
}
