// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Custom hooks for dashboard badge data (pending users, store stats)
// ABOUTME: Enables sidebar badges to share query data with panel components

import { useQuery } from '@tanstack/react-query';
import { adminApi } from '../../services/api';
import { QUERY_KEYS } from '../../constants/queryKeys';
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

/** Hook to get pending coach count for badge display */
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
