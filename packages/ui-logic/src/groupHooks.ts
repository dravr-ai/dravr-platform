// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query hooks behind Group info on both clients — the group, members, invites, settings, links
// ABOUTME: Bound to each client's groups API; creating and joining a group are commands, so no hook does either

import { useMutation, useQuery, useQueryClient, type QueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { GroupsApi } from '@pierre/api-client';
import type {
  CreateInviteRequest,
  GroupRole,
  ProposeDelegatedConnectionRequest,
  UpdateGroupRequest,
  UpdateMemberRoleRequest,
  UpdatePeerConsentRequest,
} from '@pierre/shared-types';

/**
 * How long, in milliseconds, each group read whose freshness the two clients
 * set differently stays fresh.
 *
 * Web keeps the group row and the member list fresh for 30 s and the stats for
 * 60 s; mobile keeps them for 60 s, 60 s and 5 min. Nothing in either client
 * says why they differ, so each passes its own values here rather than one
 * being chosen for both. Every other group read has one freshness, below.
 */
export interface GroupFreshness {
  /** The group row: name, description, coach, the caller's role. */
  detail: number;
  /** The member list, with each member's role and consent. */
  members: number;
  /** The group's aggregate stats. */
  stats: number;
}

/** Group reads both clients keep fresh for the same span. */
const INVITES_STALE_MS = 30_000;
const PERMISSIONS_STALE_MS = 60_000;
const DIGEST_STALE_MS = 5 * 60_000;
const TRANSCRIPT_STALE_MS = 30_000;
const LINKS_STALE_MS = 30_000;
const ROSTER_STALE_MS = 60_000;

/** Every read a TrainingPeaks link step changes: the group's links, the roster, the provider rows. */
function invalidateDelegation(queryClient: QueryClient, groupId: string) {
  return Promise.all([
    queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.delegatedConnections(groupId) }),
    queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.delegationRoster(groupId) }),
    queryClient.invalidateQueries({ queryKey: QUERY_KEYS.providers.all }),
    queryClient.invalidateQueries({ queryKey: QUERY_KEYS.providers.status() }),
    queryClient.invalidateQueries({ queryKey: QUERY_KEYS.user.providerConnections() }),
  ]);
}

/**
 * Build the group hooks over one client's groups API.
 *
 * Each client binds this once, in its own `hooks/useGroups`, with its API
 * instance and its {@link GroupFreshness}; everything else — query keys, what a
 * write refreshes, what a read returns while loading — is this one definition.
 *
 * A write whose surface stays open resolves once the reads it changed have
 * been refetched, so the sheet that made the change shows its result. Leaving
 * and archiving close the surface instead: they resolve on the server's answer
 * and refresh the lists in the background, because waiting on a refetch of a
 * group the athlete no longer sees would only hold the sheet open.
 */
export function createGroupHooks(groupsApi: GroupsApi, freshness: GroupFreshness) {
  /** A single group by ID. */
  function useGroup(groupId: string) {
    const query = useQuery({
      queryKey: QUERY_KEYS.groups.detail(groupId),
      queryFn: () => groupsApi.getGroup(groupId),
      enabled: !!groupId,
      staleTime: freshness.detail,
    });

    return {
      group: query.data ?? null,
      isLoading: query.isLoading,
      isError: query.isError,
      error: query.error,
      refetch: query.refetch,
    };
  }

  /** The member list for a group. */
  function useGroupMembers(groupId: string) {
    const query = useQuery({
      queryKey: QUERY_KEYS.groups.members(groupId),
      queryFn: () => groupsApi.listMembers(groupId),
      enabled: !!groupId,
      staleTime: freshness.members,
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
   * Aggregate stats for a group. `enabled` is false for the group's coach,
   * whom the stats route does not admit.
   */
  function useGroupStats(groupId: string, enabled = true) {
    const query = useQuery({
      queryKey: QUERY_KEYS.groups.stats(groupId),
      queryFn: () => groupsApi.getStats(groupId),
      enabled: !!groupId && enabled,
      staleTime: freshness.stats,
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
   * The invites issued for a group. `enabled` is false for the group's coach,
   * whom the invites route does not admit.
   */
  function useGroupInvites(groupId: string, enabled = true) {
    const query = useQuery({
      queryKey: QUERY_KEYS.groups.invites(groupId),
      queryFn: () => groupsApi.listInvites(groupId),
      enabled: !!groupId && enabled,
      staleTime: INVITES_STALE_MS,
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
   * Issue an invite code (admin/owner only).
   *
   * The code is what an athlete redeems with `/group join <code>`, and what the
   * `/groups/join/:code` link carries, so the surface that creates one is also
   * where it gets shared from.
   */
  function useCreateInvite(groupId: string) {
    const queryClient = useQueryClient();

    const mutation = useMutation({
      mutationFn: (request?: CreateInviteRequest) => groupsApi.createInvite(groupId, request),
      onSuccess: () => queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.invites(groupId) }),
    });

    return {
      createInvite: mutation.mutateAsync,
      isPending: mutation.isPending,
      isError: mutation.isError,
      error: mutation.error,
    };
  }

  /** Deactivate an invite (admin/owner only). */
  function useDeactivateInvite(groupId: string) {
    const queryClient = useQueryClient();

    const mutation = useMutation({
      mutationFn: (inviteId: string) => groupsApi.deactivateInvite(groupId, inviteId),
      onSuccess: () => queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.invites(groupId) }),
    });

    return {
      deactivateInvite: mutation.mutateAsync,
      isPending: mutation.isPending,
      isError: mutation.isError,
      error: mutation.error,
    };
  }

  /** Update a group's settings (admin/owner only). */
  function useUpdateGroup(groupId: string) {
    const queryClient = useQueryClient();

    const mutation = useMutation({
      mutationFn: (request: UpdateGroupRequest) => groupsApi.updateGroup(groupId, request),
      onSuccess: () =>
        Promise.all([
          queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.detail(groupId) }),
          // A renamed group renames the row that names it in the list.
          queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() }),
        ]),
    });

    return {
      updateGroup: mutation.mutateAsync,
      isPending: mutation.isPending,
      isError: mutation.isError,
      error: mutation.error,
    };
  }

  /** Archive a group (owner only). */
  function useDeleteGroup() {
    const queryClient = useQueryClient();

    const mutation = useMutation({
      mutationFn: (groupId: string) => groupsApi.deleteGroup(groupId),
      onSuccess: () => {
        void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.all });
        void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });
      },
    });

    return {
      deleteGroup: mutation.mutateAsync,
      isPending: mutation.isPending,
      isError: mutation.isError,
      error: mutation.error,
    };
  }

  /** Leave a group. */
  function useLeaveGroup() {
    const queryClient = useQueryClient();

    const mutation = useMutation({
      mutationFn: (groupId: string) => groupsApi.leaveGroup(groupId),
      onSuccess: () => {
        void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.all });
        // Leaving strands the member's group-scoped thread, so the list is the
        // other half of the answer: it is refetched, not patched.
        void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });
      },
    });

    return {
      leaveGroup: mutation.mutateAsync,
      isPending: mutation.isPending,
      isError: mutation.isError,
      error: mutation.error,
    };
  }

  /** Remove a member from a group (admin/owner only). */
  function useRemoveMember(groupId: string) {
    const queryClient = useQueryClient();

    const mutation = useMutation({
      mutationFn: (userId: string) => groupsApi.removeMember(groupId, userId),
      onSuccess: () =>
        Promise.all([
          queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.members(groupId) }),
          // The stats are aggregated over the members, so one fewer changes them.
          queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.stats(groupId) }),
        ]),
    });

    return {
      removeMember: mutation.mutateAsync,
      isPending: mutation.isPending,
      isError: mutation.isError,
      error: mutation.error,
    };
  }

  /** Detach the group's human coach (admin/owner only). */
  function useRemoveCoach(groupId: string) {
    const queryClient = useQueryClient();

    const mutation = useMutation({
      mutationFn: () => groupsApi.removeCoach(groupId),
      onSuccess: () => queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.detail(groupId) }),
    });

    return {
      removeCoach: mutation.mutateAsync,
      isPending: mutation.isPending,
      isError: mutation.isError,
      error: mutation.error,
    };
  }

  /** Promote or demote a member (owner only). */
  function useUpdateMemberRole(groupId: string) {
    const queryClient = useQueryClient();

    const mutation = useMutation({
      mutationFn: ({ userId, role }: { userId: string; role: GroupRole }) => {
        const request: UpdateMemberRoleRequest = { role };
        return groupsApi.updateMemberRole(groupId, userId, request);
      },
      onSuccess: () => queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.members(groupId) }),
    });

    return {
      updateRole: mutation.mutateAsync,
      isPending: mutation.isPending,
      isError: mutation.isError,
      error: mutation.error,
    };
  }

  /**
   * The caller's own peer-sharing consent in a group.
   *
   * This is the GDPR-relevant control: it decides whether the athlete's own
   * training data is readable by the group's other members and by the group
   * coach. It applies to the caller's membership row only — there is no path
   * here to change anyone else's consent.
   */
  function useUpdatePeerConsent(groupId: string) {
    const queryClient = useQueryClient();

    const mutation = useMutation({
      mutationFn: (consent: boolean) => {
        const request: UpdatePeerConsentRequest = { consent };
        return groupsApi.updatePeerConsent(groupId, request);
      },
      onSuccess: () => queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.members(groupId) }),
    });

    return {
      updateConsent: mutation.mutateAsync,
      isPending: mutation.isPending,
      isError: mutation.isError,
      error: mutation.error,
    };
  }

  /**
   * The tenant's group permissions and tier flags.
   *
   * `weeklyDigest` is the plan-tier flag resolved server-side — the same read
   * the digest scheduler sweeps on. Surfaces that render the weekly report or
   * health flags gate on it rather than deriving a tier locally.
   */
  function useGroupPermissions() {
    const query = useQuery({
      queryKey: QUERY_KEYS.groups.permissions(),
      queryFn: () => groupsApi.getPermissions(),
      staleTime: PERMISSIONS_STALE_MS,
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
   * A group's weekly report. Admin/owner only server-side, and only worth
   * asking for when the tenant's tier enables the weekly digest — both
   * conditions ride in through `enabled` so a member never fires a request the
   * server will refuse.
   */
  function useGroupWeeklyReport(groupId: string, enabled: boolean) {
    const query = useQuery({
      queryKey: QUERY_KEYS.groups.report(groupId),
      queryFn: () => groupsApi.getWeeklyReport(groupId),
      enabled: !!groupId && enabled,
      staleTime: DIGEST_STALE_MS,
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
   * The health flags raised for a group's members. Same admin + tier gate as
   * `useGroupWeeklyReport`.
   */
  function useGroupHealthFlags(groupId: string, enabled: boolean) {
    const query = useQuery({
      queryKey: QUERY_KEYS.groups.health(groupId),
      queryFn: () => groupsApi.getHealthFlags(groupId),
      enabled: !!groupId && enabled,
      staleTime: DIGEST_STALE_MS,
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
   * The group's shared room transcript.
   *
   * Membership-gated server-side and consent-filtered in the same SQL the
   * pipeline's ambient context uses, so what a member sees here is exactly what
   * the agent reasons from — one visibility rule for humans and model alike.
   */
  function useGroupTranscript(groupId: string, enabled: boolean) {
    const query = useQuery({
      queryKey: QUERY_KEYS.groups.transcript(groupId),
      queryFn: () => groupsApi.getTranscript(groupId),
      enabled: !!groupId && enabled,
      staleTime: TRANSCRIPT_STALE_MS,
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
  function useDelegatedConnections(groupId: string, enabled: boolean) {
    const query = useQuery({
      queryKey: QUERY_KEYS.groups.delegatedConnections(groupId),
      queryFn: () => groupsApi.listDelegatedConnections(groupId),
      enabled: !!groupId && enabled,
      staleTime: LINKS_STALE_MS,
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
  function useDelegationRoster(groupId: string, enabled: boolean) {
    const query = useQuery({
      queryKey: QUERY_KEYS.groups.delegationRoster(groupId),
      queryFn: () => groupsApi.getDelegationRoster(groupId),
      enabled: !!groupId && enabled,
      staleTime: ROSTER_STALE_MS,
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
   * the answer where `useDelegationRoster` reads it.
   */
  function useRefreshDelegationRoster(groupId: string) {
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

  /** The coach links a roster athlete to a live member; the member then confirms. */
  function useProposeDelegatedConnection(groupId: string) {
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
  function useConfirmDelegatedConnection(groupId: string) {
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
  function useEndDelegatedConnection(groupId: string) {
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

  return {
    useGroup,
    useGroupMembers,
    useGroupStats,
    useGroupInvites,
    useCreateInvite,
    useDeactivateInvite,
    useUpdateGroup,
    useDeleteGroup,
    useLeaveGroup,
    useRemoveMember,
    useRemoveCoach,
    useUpdateMemberRole,
    useUpdatePeerConsent,
    useGroupPermissions,
    useGroupWeeklyReport,
    useGroupHealthFlags,
    useGroupTranscript,
    useDelegatedConnections,
    useDelegationRoster,
    useRefreshDelegationRoster,
    useProposeDelegatedConnection,
    useConfirmDelegatedConnection,
    useEndDelegatedConnection,
  };
}
