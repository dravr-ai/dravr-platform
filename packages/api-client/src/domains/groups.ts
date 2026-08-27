// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Groups domain API - read, manage, invite and leave a coaching group
// ABOUTME: Creating and joining a group are the /group create and /group join commands

import type { AxiosInstance } from 'axios';
import type {
  CoachingGroup,
  UpdateGroupRequest,
  UpdateMemberRoleRequest,
  UpdatePeerConsentRequest,
  CreateInviteRequest,
  GroupMember,
  GroupInvite,
  GroupAggregateStats,
  GroupWeeklyReport,
  GroupHealthFlag,
  GroupMembersResponse,
  GroupInvitesResponse,
  GroupStatsResponse,
  GroupWeeklyReportResponse,
  GroupHealthFlagsResponse,
  GroupPermissionsResponse,
  GroupTranscriptResponse,
} from '@pierre/shared-types';
import { ENDPOINTS } from '../core/endpoints';

// Re-export types for consumers
export type {
  CoachingGroup,
  UpdateGroupRequest,
  GroupMember,
  GroupInvite,
  GroupAggregateStats,
  GroupWeeklyReport,
  GroupHealthFlag,
  GroupMembersResponse,
  GroupInvitesResponse,
  GroupStatsResponse,
  GroupWeeklyReportResponse,
  GroupHealthFlagsResponse,
  GroupPermissionsResponse,
};

/**
 * Creates the groups API methods bound to an axios instance.
 */
export function createGroupsApi(axios: AxiosInstance) {
  return {
    // ==================== GROUP CRUD ====================

    /** Get a specific group by ID */
    async getGroup(groupId: string): Promise<CoachingGroup> {
      const response = await axios.get<CoachingGroup>(ENDPOINTS.GROUPS.GROUP(groupId));
      return response.data;
    },

    /** Update a group (admin/owner only) */
    async updateGroup(groupId: string, request: UpdateGroupRequest): Promise<CoachingGroup> {
      const response = await axios.put<CoachingGroup>(ENDPOINTS.GROUPS.GROUP(groupId), request);
      return response.data;
    },

    /** Delete (archive) a group (owner only) */
    async deleteGroup(groupId: string): Promise<void> {
      await axios.delete(ENDPOINTS.GROUPS.GROUP(groupId));
    },

    // ==================== MEMBERSHIP ====================

    /** Leave a group */
    async leaveGroup(groupId: string): Promise<void> {
      await axios.post(ENDPOINTS.GROUPS.LEAVE(groupId));
    },

    // ==================== HUMAN COACH ====================

    /**
     * Detach the group's human coach (admin/owner only). The coach is invited
     * by issuing a `kind: 'coach'` invite via {@link createInvite}; redemption
     * goes through the `/group join <code>` command like any other invite code.
     */
    async removeCoach(groupId: string): Promise<void> {
      await axios.delete(ENDPOINTS.GROUPS.COACH(groupId));
    },

    /** List members of a group */
    async listMembers(groupId: string): Promise<GroupMembersResponse> {
      const response = await axios.get<GroupMembersResponse>(ENDPOINTS.GROUPS.MEMBERS(groupId));
      return response.data;
    },

    /** Remove a member from a group (admin/owner only) */
    async removeMember(groupId: string, userId: string): Promise<void> {
      await axios.delete(ENDPOINTS.GROUPS.MEMBER(groupId, userId));
    },

    /** Update a member's role (owner only) */
    async updateMemberRole(
      groupId: string,
      userId: string,
      request: UpdateMemberRoleRequest,
    ): Promise<void> {
      await axios.put(ENDPOINTS.GROUPS.MEMBER_ROLE(groupId, userId), request);
    },

    /** Update own peer sharing consent */
    async updatePeerConsent(groupId: string, request: UpdatePeerConsentRequest): Promise<void> {
      await axios.put(ENDPOINTS.GROUPS.MY_CONSENT(groupId), request);
    },

    // ==================== INVITES ====================

    /** Create a new invite for a group (admin/owner only) */
    async createInvite(groupId: string, request?: CreateInviteRequest): Promise<GroupInvite> {
      const response = await axios.post<GroupInvite>(ENDPOINTS.GROUPS.INVITES(groupId), request);
      return response.data;
    },

    /** List invites for a group */
    async listInvites(groupId: string): Promise<GroupInvitesResponse> {
      const response = await axios.get<GroupInvitesResponse>(ENDPOINTS.GROUPS.INVITES(groupId));
      return response.data;
    },

    /** Deactivate an invite (admin/owner only) */
    async deactivateInvite(groupId: string, inviteId: string): Promise<void> {
      await axios.delete(ENDPOINTS.GROUPS.INVITE(groupId, inviteId));
    },

    // ==================== PERMISSIONS ====================

    /** Check if the current user can create groups */
    async getPermissions(): Promise<GroupPermissionsResponse> {
      const response = await axios.get<GroupPermissionsResponse>(ENDPOINTS.GROUPS.PERMISSIONS);
      return response.data;
    },

    // ==================== ANALYTICS ====================

    /** Get aggregate stats for a group */
    async getStats(groupId: string): Promise<GroupStatsResponse> {
      const response = await axios.get<GroupStatsResponse>(ENDPOINTS.GROUPS.STATS(groupId));
      return response.data;
    },

    /**
     * Get the weekly report for a group (admin/owner only).
     *
     * The route wraps the report in `{ report, metadata }`, mirroring
     * {@link getStats}; the wrapper is what comes back so callers read the
     * same shape the server sends.
     */
    async getWeeklyReport(groupId: string): Promise<GroupWeeklyReportResponse> {
      const response = await axios.get<GroupWeeklyReportResponse>(
        ENDPOINTS.GROUPS.REPORT(groupId),
      );
      return response.data;
    },

    /** Get health flags for a group's members (admin/owner only) */
    async getHealthFlags(groupId: string): Promise<GroupHealthFlagsResponse> {
      const response = await axios.get<GroupHealthFlagsResponse>(ENDPOINTS.GROUPS.HEALTH(groupId));
      return response.data;
    },

    /**
     * Read the group's shared room transcript as the authenticated member.
     *
     * The server withholds an unconsented member's entries while keeping them
     * on the roster, so the caller renders exactly what the pipeline's own
     * ambient context sees -- one visibility rule, every surface.
     */
    async getTranscript(groupId: string, limit?: number): Promise<GroupTranscriptResponse> {
      const response = await axios.get<GroupTranscriptResponse>(
        ENDPOINTS.GROUPS.TRANSCRIPT(groupId),
        limit === undefined ? undefined : { params: { limit } },
      );
      return response.data;
    },
  };
}

export type GroupsApi = ReturnType<typeof createGroupsApi>;
