// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query hooks for group coaching operations
// ABOUTME: Provides queries and mutations for groups, members, invites, and analytics

import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { useCallback } from 'react';
import { QUERY_KEYS } from '../../../packages/shared-constants/src/query-keys';
import { groupsApi } from '../services/api';
import type {
  CreateGroupRequest,
  GroupRole,
  JoinGroupRequest,
  UpdateGroupRequest,
  UpdateMemberRoleRequest,
  UpdatePeerConsentRequest,
} from '../types';

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

/**
 * Hook for fetching the invites issued for a group.
 */
export function useGroupInvites(groupId: string) {
  const query = useQuery({
    queryKey: QUERY_KEYS.groups.invites(groupId),
    queryFn: () => groupsApi.listInvites(groupId),
    staleTime: 30_000,
    enabled: !!groupId,
  });

  return {
    invites: query.data?.invites ?? [],
    isLoading: query.isLoading,
    isError: query.isError,
    error: query.error,
    refetch: query.refetch,
  };
}

/**
 * Mutation hook for updating a group's settings (admin/owner only).
 */
export function useUpdateGroup(groupId: string) {
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: (request: UpdateGroupRequest) => groupsApi.updateGroup(groupId, request),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.detail(groupId) });
      await queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.list() });
    },
  });

  return {
    updateGroup: mutation.mutateAsync,
    isPending: mutation.isPending,
    isError: mutation.isError,
    error: mutation.error,
  };
}

/**
 * Mutation hook for archiving a group (owner only).
 */
export function useDeleteGroup() {
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: (groupId: string) => groupsApi.deleteGroup(groupId),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.all });
    },
  });

  return {
    deleteGroup: mutation.mutateAsync,
    isPending: mutation.isPending,
    isError: mutation.isError,
    error: mutation.error,
  };
}

/**
 * Mutation hook for deactivating an invite (admin/owner only).
 */
export function useDeactivateInvite(groupId: string) {
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: (inviteId: string) => groupsApi.deactivateInvite(groupId, inviteId),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.invites(groupId) });
    },
  });

  return {
    deactivateInvite: mutation.mutateAsync,
    isPending: mutation.isPending,
    isError: mutation.isError,
    error: mutation.error,
  };
}

/**
 * Mutation hook for promoting or demoting a member (owner only).
 */
export function useUpdateMemberRole(groupId: string) {
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: ({ userId, role }: { userId: string; role: GroupRole }) => {
      const request: UpdateMemberRoleRequest = { role };
      return groupsApi.updateMemberRole(groupId, userId, request);
    },
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.members(groupId) });
    },
  });

  return {
    updateRole: mutation.mutateAsync,
    isPending: mutation.isPending,
    isError: mutation.isError,
    error: mutation.error,
  };
}

/**
 * Hook for the tenant's group permissions and tier flags.
 *
 * `weeklyDigest` is the plan-tier flag resolved server-side — the same read
 * the digest scheduler sweeps on. Surfaces that render the weekly report or
 * health flags gate on it rather than deriving a tier locally.
 */
export function useGroupPermissions() {
  const query = useQuery({
    queryKey: QUERY_KEYS.groups.permissions(),
    queryFn: () => groupsApi.getPermissions(),
    staleTime: 60_000,
  });

  return {
    canCreate: query.data?.can_create ?? true,
    policy: query.data?.policy ?? 'everyone',
    weeklyDigest: query.data?.weekly_digest ?? false,
    isLoading: query.isLoading,
    isError: query.isError,
  };
}

/**
 * Hook for a group's weekly report. Admin/owner only server-side, and only
 * worth asking for when the tenant's tier enables the weekly digest — both
 * conditions ride in through `enabled` so a member never fires a request the
 * server will refuse.
 */
export function useGroupWeeklyReport(groupId: string, enabled: boolean) {
  const query = useQuery({
    queryKey: QUERY_KEYS.groups.report(groupId),
    queryFn: () => groupsApi.getWeeklyReport(groupId),
    staleTime: 5 * 60_000,
    enabled: !!groupId && enabled,
  });

  return {
    report: query.data?.report ?? null,
    isLoading: query.isLoading,
    isError: query.isError,
    error: query.error,
    refetch: query.refetch,
  };
}

/**
 * Hook for the health flags raised for a group's members. Same admin + tier
 * gate as {@link useGroupWeeklyReport}.
 */
export function useGroupHealthFlags(groupId: string, enabled: boolean) {
  const query = useQuery({
    queryKey: QUERY_KEYS.groups.health(groupId),
    queryFn: () => groupsApi.getHealthFlags(groupId),
    staleTime: 5 * 60_000,
    enabled: !!groupId && enabled,
  });

  return {
    flags: query.data?.flags ?? [],
    total: query.data?.total ?? 0,
    isLoading: query.isLoading,
    isError: query.isError,
    error: query.error,
    refetch: query.refetch,
  };
}

/**
 * Mutation hook for the caller's own peer-sharing consent in a group.
 *
 * This is the GDPR-relevant control: it decides whether the athlete's own
 * training data is readable by the group's other members and by the group
 * coach. It applies to the caller's membership row only — there is no path
 * here to change anyone else's consent.
 */
export function useUpdatePeerConsent(groupId: string) {
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: (consent: boolean) => {
      const request: UpdatePeerConsentRequest = { consent };
      return groupsApi.updatePeerConsent(groupId, request);
    },
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.members(groupId) });
    },
  });

  return {
    updateConsent: mutation.mutateAsync,
    isPending: mutation.isPending,
    isError: mutation.isError,
    error: mutation.error,
  };
}

/**
 * Reads the group's shared room transcript.
 *
 * Mirrors the web hook: membership-gated server-side, consent-filtered in the
 * same SQL the pipeline's ambient context reads, so the member and the coach
 * see one room.
 */
export function useGroupTranscript(groupId: string, enabled: boolean) {
  const query = useQuery({
    queryKey: QUERY_KEYS.groups.transcript(groupId),
    queryFn: () => groupsApi.getTranscript(groupId),
    enabled: !!groupId && enabled,
    staleTime: 30_000,
  });

  return {
    transcript: query.data ?? null,
    isLoading: query.isLoading,
    isError: query.isError,
    error: query.error,
    refetch: query.refetch,
  };
}
