// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query hooks for group coaching operations
// ABOUTME: Provides queries and mutations for groups, members, invites, and analytics

import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { useCallback } from 'react';
import { QUERY_KEYS } from '../../../packages/shared-constants/src/query-keys';
import { groupsApi } from '../services/api';
import type { CreateGroupRequest, JoinGroupRequest } from '../types';

/**
 * Hook for fetching groups the current user belongs to.
 */
export function useMyGroups() {
  const queryClient = useQueryClient();

  const query = useQuery({
    queryKey: QUERY_KEYS.groups.list(),
    queryFn: () => groupsApi.listMyGroups(),
    staleTime: 60_000, // 1 minute
  });

  const invalidate = useCallback(async () => {
    await queryClient.invalidateQueries({
      queryKey: QUERY_KEYS.groups.all,
    });
  }, [queryClient]);

  return {
    groups: query.data?.groups ?? [],
    isLoading: query.isLoading,
    isRefetching: query.isRefetching,
    isError: query.isError,
    error: query.error,
    refetch: query.refetch,
    invalidate,
  };
}

/**
 * Hook for fetching a single group by ID.
 */
export function useGroup(groupId: string) {
  const query = useQuery({
    queryKey: QUERY_KEYS.groups.detail(groupId),
    queryFn: () => groupsApi.getGroup(groupId),
    staleTime: 60_000,
    enabled: !!groupId,
  });

  return {
    group: query.data ?? null,
    isLoading: query.isLoading,
    isError: query.isError,
    error: query.error,
    refetch: query.refetch,
  };
}

/**
 * Hook for fetching members of a group.
 */
export function useGroupMembers(groupId: string) {
  const query = useQuery({
    queryKey: QUERY_KEYS.groups.members(groupId),
    queryFn: () => groupsApi.listMembers(groupId),
    staleTime: 60_000,
    enabled: !!groupId,
  });

  return {
    members: query.data?.members ?? [],
    isLoading: query.isLoading,
    isError: query.isError,
    error: query.error,
    refetch: query.refetch,
  };
}

/**
 * Hook for fetching group aggregate stats.
 */
export function useGroupStats(groupId: string) {
  const query = useQuery({
    queryKey: QUERY_KEYS.groups.stats(groupId),
    queryFn: () => groupsApi.getStats(groupId),
    staleTime: 5 * 60_000, // 5 minutes
    enabled: !!groupId,
  });

  return {
    stats: query.data?.stats ?? null,
    isLoading: query.isLoading,
    isError: query.isError,
    error: query.error,
    refetch: query.refetch,
  };
}

/**
 * Hook for group mutation operations (create, join, leave).
 */
export function useGroupActions() {
  const queryClient = useQueryClient();

  const invalidateAll = useCallback(async () => {
    await queryClient.invalidateQueries({
      queryKey: QUERY_KEYS.groups.all,
    });
  }, [queryClient]);

  const createGroup = useMutation({
    mutationFn: (request: CreateGroupRequest) => groupsApi.createGroup(request),
    onSuccess: invalidateAll,
  });

  const joinGroup = useMutation({
    mutationFn: (request: JoinGroupRequest) => groupsApi.joinGroup(request),
    onSuccess: invalidateAll,
  });

  const leaveGroup = useMutation({
    mutationFn: (groupId: string) => groupsApi.leaveGroup(groupId),
    onSuccess: invalidateAll,
  });

  return {
    createGroup: createGroup.mutateAsync,
    isCreating: createGroup.isPending,
    createError: createGroup.error,

    joinGroup: joinGroup.mutateAsync,
    isJoining: joinGroup.isPending,
    joinError: joinGroup.error,

    leaveGroup: leaveGroup.mutateAsync,
    isLeaving: leaveGroup.isPending,
    leaveError: leaveGroup.error,
  };
}
