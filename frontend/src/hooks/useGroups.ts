// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query hooks for the group surfaces that live inside chat — Group info and admin settings
// ABOUTME: Creating and joining a group are `/group create|join` commands, so no hook here writes a group

import { useQuery, useMutation, useQueryClient, type QueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import { groupsApi } from '../services/api';
import type {
  CreateInviteRequest,
  UpdateGroupRequest,
  UpdateMemberRoleRequest,
  UpdatePeerConsentRequest,
  ProposeDelegatedConnectionRequest,
  GroupRole,
} from '@pierre/shared-types';

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
 * Fetches aggregate stats for a group. `enabled` is false for the group's
 * coach, whom the stats route does not admit.
 */
export function useGroupStats(groupId: string, enabled = true) {
  const query = useQuery({
    queryKey: QUERY_KEYS.groups.stats(groupId),
    queryFn: () => groupsApi.getStats(groupId),
    enabled: !!groupId && enabled,
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
 * Fetches invites for a group. `enabled` is false for the group's coach,
 * whom the invites route does not admit.
 */
export function useGroupInvites(groupId: string, enabled = true) {
  const query = useQuery({
    queryKey: QUERY_KEYS.groups.invites(groupId),
    queryFn: () => groupsApi.listInvites(groupId),
    enabled: !!groupId && enabled,
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
 * Mutation hook for detaching a group's human coach (admin/owner only).
 */
export function useRemoveCoach(groupId: string) {
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: () => groupsApi.removeCoach(groupId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.detail(groupId) });
    },
  });

  return {
    removeCoach: mutation.mutateAsync,
    isPending: mutation.isPending,
    isError: mutation.isError,
    error: mutation.error,
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
      // Leaving strands the member's group-scoped thread, so the list is the
      // other half of the answer: it is refetched, not patched.
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });
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
      // A renamed group renames the row that names it in the list.
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });
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
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });
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
 * Fetches group creation permissions for the current user.
 *
 * `weeklyDigest` is the tenant's plan-tier flag, resolved server-side from the
 * same read the digest scheduler sweeps on. Surfaces that render the weekly
 * report or health flags gate on it rather than deriving a tier locally.
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
 * Fetches the group's weekly report. Admin/owner only server-side, and only
 * worth asking for when the tenant's tier enables the weekly digest — pass
 * both conditions through `enabled` so a member never fires a request the
 * server will refuse.
 */
export function useGroupWeeklyReport(groupId: string, enabled: boolean) {
  const query = useQuery({
    queryKey: QUERY_KEYS.groups.report(groupId),
    queryFn: () => groupsApi.getWeeklyReport(groupId),
    enabled: !!groupId && enabled,
    staleTime: 5 * 60_000,
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
 * Fetches the health flags raised for the group's members. Same admin + tier
 * gate as {@link useGroupWeeklyReport}.
 */
export function useGroupHealthFlags(groupId: string, enabled: boolean) {
  const query = useQuery({
    queryKey: QUERY_KEYS.groups.health(groupId),
    queryFn: () => groupsApi.getHealthFlags(groupId),
    enabled: !!groupId && enabled,
    staleTime: 5 * 60_000,
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
 * Reads the group's shared room transcript.
 *
 * Membership-gated server-side and consent-filtered in the same SQL the
 * pipeline's ambient context uses, so what a member sees here is exactly what
 * the agent reasons from -- one visibility rule for humans and model alike.
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

/**
 * The group's live TrainingPeaks links: every one for the group's coach, only
 * their own for a member (`viewer` says which). Asked only for a group with a
 * human coach, since a link needs one.
 */
export function useDelegatedConnections(groupId: string, enabled: boolean) {
  const query = useQuery({
    queryKey: QUERY_KEYS.groups.delegatedConnections(groupId),
    queryFn: () => groupsApi.listDelegatedConnections(groupId),
    enabled: !!groupId && enabled,
    staleTime: 30_000,
  });

  return {
    connections: query.data?.connections ?? [],
    viewer: query.data?.viewer ?? null,
    isLoading: query.isLoading,
    isError: query.isError,
    error: query.error,
    refetch: query.refetch,
  };
}

/**
 * The coach's TrainingPeaks roster, each athlete with its link in this group.
 * The coach alone may read it. A refusal (not connected, not a coach account,
 * reconnect needed, notice outdated) is an answer, not a blip, so it is not
 * retried: the section words `details.reason` instead.
 */
export function useDelegationRoster(groupId: string, enabled: boolean) {
  const query = useQuery({
    queryKey: QUERY_KEYS.groups.delegationRoster(groupId),
    queryFn: () => groupsApi.getDelegationRoster(groupId),
    enabled: !!groupId && enabled,
    staleTime: 60_000,
    retry: false,
  });

  return {
    athletes: query.data?.athletes ?? [],
    isLoading: query.isLoading,
    isError: query.isError,
    error: query.error,
    isRefreshing: query.isRefetching,
  };
}

/**
 * Read the coach's roster live, past the server's ten-minute cache, and put
 * the answer where {@link useDelegationRoster} reads it.
 */
export function useRefreshDelegationRoster(groupId: string) {
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: () => groupsApi.getDelegationRoster(groupId, { refresh: true }),
    onSuccess: (roster) => {
      queryClient.setQueryData(QUERY_KEYS.groups.delegationRoster(groupId), roster);
    },
  });

  return {
    refreshRoster: mutation.mutateAsync,
    isPending: mutation.isPending,
  };
}

/** Every read a link step changes: the group's links, the roster, the provider rows. */
function invalidateDelegation(queryClient: QueryClient, groupId: string) {
  return Promise.all([
    queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.delegatedConnections(groupId) }),
    queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.delegationRoster(groupId) }),
    queryClient.invalidateQueries({ queryKey: QUERY_KEYS.providers.all }),
    queryClient.invalidateQueries({ queryKey: QUERY_KEYS.providers.status() }),
    queryClient.invalidateQueries({ queryKey: QUERY_KEYS.user.providerConnections() }),
  ]);
}

/** The coach links a roster athlete to a live member; the member then confirms. */
export function useProposeDelegatedConnection(groupId: string) {
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: ({ athleteId, memberUserId }: { athleteId: string; memberUserId: string }) => {
      const request: ProposeDelegatedConnectionRequest = {
        provider: 'trainingpeaks',
        provider_athlete_id: athleteId,
        member_user_id: memberUserId,
      };
      return groupsApi.proposeDelegatedConnection(groupId, request);
    },
    onSuccess: () => invalidateDelegation(queryClient, groupId),
  });

  return {
    proposeLink: mutation.mutateAsync,
    isPending: mutation.isPending,
  };
}

/**
 * The member confirms a proposed link: their consent to having their
 * TrainingPeaks workouts read through the coach's account.
 */
export function useConfirmDelegatedConnection(groupId: string) {
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: (connectionId: string) => groupsApi.confirmDelegatedConnection(groupId, connectionId),
    onSuccess: () => invalidateDelegation(queryClient, groupId),
  });

  return {
    confirmLink: mutation.mutateAsync,
    isPending: mutation.isPending,
  };
}

/**
 * End a link from either side: the member declines or unlinks, the coach
 * withdraws or unlinks. The server derives which from the caller.
 */
export function useEndDelegatedConnection(groupId: string) {
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: (connectionId: string) => groupsApi.endDelegatedConnection(groupId, connectionId),
    onSuccess: () => invalidateDelegation(queryClient, groupId),
  });

  return {
    endLink: mutation.mutateAsync,
    isPending: mutation.isPending,
  };
}
