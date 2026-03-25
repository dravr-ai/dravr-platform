// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query hooks for group coaching management on the web frontend
// ABOUTME: Provides queries for groups, members, stats, invites and mutations for CRUD operations

import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { useCallback } from 'react';
import { QUERY_KEYS } from '@pierre/shared-constants';
import { groupsApi } from '../services/api';
import type {
  CreateGroupRequest,
  JoinGroupRequest,
  CreateInviteRequest,
  UpdateGroupRequest,
  UpdateMemberRoleRequest,
  UpdatePeerConsentRequest,
  GroupRole,
} from '@pierre/shared-types';

/**
 * Fetches the list of groups the current user belongs to.
 */
export function useMyGroups() {
  const query = useQuery({
    queryKey: QUERY_KEYS.groups.list(),
    queryFn: () => groupsApi.listMyGroups(),
    staleTime: 30_000,
  });

  return {
    groups: query.data?.groups ?? [],
    isLoading: query.isLoading,
    isError: query.isError,
    error: query.error,
    refetch: query.refetch,
  };
}

/**
 * Fetches a single group by ID.
 */
export function useGroup(groupId: string) {
  const query = useQuery({
    queryKey: QUERY_KEYS.groups.detail(groupId),
    queryFn: () => groupsApi.getGroup(groupId),
    enabled: !!groupId,
    staleTime: 30_000,
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
 * Fetches the member list for a group.
 */
export function useGroupMembers(groupId: string) {
  const query = useQuery({
    queryKey: QUERY_KEYS.groups.members(groupId),
    queryFn: () => groupsApi.listMembers(groupId),
    enabled: !!groupId,
    staleTime: 30_000,
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
 * Fetches aggregate stats for a group.
 */
export function useGroupStats(groupId: string) {
  const query = useQuery({
    queryKey: QUERY_KEYS.groups.stats(groupId),
    queryFn: () => groupsApi.getStats(groupId),
    enabled: !!groupId,
    staleTime: 60_000,
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
 * Fetches invites for a group.
 */
export function useGroupInvites(groupId: string) {
  const query = useQuery({
    queryKey: QUERY_KEYS.groups.invites(groupId),
    queryFn: () => groupsApi.listInvites(groupId),
    enabled: !!groupId,
    staleTime: 30_000,
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
 * Mutation hook for creating a new coaching group.
 */
export function useCreateGroup() {
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: (request: CreateGroupRequest) => groupsApi.createGroup(request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.all });
    },
  });

  return {
    createGroup: mutation.mutateAsync,
    isPending: mutation.isPending,
    isError: mutation.isError,
    error: mutation.error,
    reset: mutation.reset,
  };
}

/**
 * Mutation hook for joining a group via invite code.
 */
export function useJoinGroup() {
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: (request: JoinGroupRequest) => groupsApi.joinGroup(request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.all });
    },
  });

  return {
    joinGroup: mutation.mutateAsync,
    isPending: mutation.isPending,
    isError: mutation.isError,
    error: mutation.error,
    reset: mutation.reset,
  };
}

/**
 * Mutation hook for leaving a group.
 */
export function useLeaveGroup() {
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: (groupId: string) => groupsApi.leaveGroup(groupId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.all });
    },
  });

  return {
    leaveGroup: mutation.mutateAsync,
    isPending: mutation.isPending,
    isError: mutation.isError,
    error: mutation.error,
  };
}

/**
 * Mutation hook for updating a group (admin/owner only).
 */
export function useUpdateGroup(groupId: string) {
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: (request: UpdateGroupRequest) => groupsApi.updateGroup(groupId, request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.detail(groupId) });
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.list() });
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
 * Mutation hook for deleting (archiving) a group (owner only).
 */
export function useDeleteGroup() {
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: (groupId: string) => groupsApi.deleteGroup(groupId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.all });
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
 * Mutation hook for creating an invite for a group.
 */
export function useCreateInvite(groupId: string) {
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: (request?: CreateInviteRequest) => groupsApi.createInvite(groupId, request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.invites(groupId) });
    },
  });

  return {
    createInvite: mutation.mutateAsync,
    isPending: mutation.isPending,
    isError: mutation.isError,
    error: mutation.error,
  };
}

/**
 * Mutation hook for deactivating an invite.
 */
export function useDeactivateInvite(groupId: string) {
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: (inviteId: string) => groupsApi.deactivateInvite(groupId, inviteId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.invites(groupId) });
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
 * Mutation hook for removing a member from a group (admin/owner only).
 */
export function useRemoveMember(groupId: string) {
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: (userId: string) => groupsApi.removeMember(groupId, userId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.members(groupId) });
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.stats(groupId) });
    },
  });

  return {
    removeMember: mutation.mutateAsync,
    isPending: mutation.isPending,
    isError: mutation.isError,
    error: mutation.error,
  };
}

/**
 * Mutation hook for updating a member's role (owner only).
 */
export function useUpdateMemberRole(groupId: string) {
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: ({ userId, role }: { userId: string; role: GroupRole }) => {
      const request: UpdateMemberRoleRequest = { role };
      return groupsApi.updateMemberRole(groupId, userId, request);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.members(groupId) });
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
 * Mutation hook for updating own peer sharing consent.
 */
export function useUpdatePeerConsent(groupId: string) {
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: (consent: boolean) => {
      const request: UpdatePeerConsentRequest = { consent };
      return groupsApi.updatePeerConsent(groupId, request);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.members(groupId) });
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
 * Invalidates all group-related queries.
 * Useful after bulk operations or navigation changes.
 */
export function useInvalidateGroups() {
  const queryClient = useQueryClient();

  return useCallback(() => {
    queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.all });
  }, [queryClient]);
}
