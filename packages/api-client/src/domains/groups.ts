// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Groups domain API - CRUD, membership, invites, analytics
// ABOUTME: Manages coaching groups with multi-person AI coaching support

import type { AxiosInstance } from 'axios';
import type {
  CoachingGroup,
  CreateGroupRequest,
  UpdateGroupRequest,
  JoinGroupRequest,
  UpdateMemberRoleRequest,
  UpdatePeerConsentRequest,
  CreateInviteRequest,
  GroupSummary,
  GroupMember,
  GroupInvite,
  GroupAggregateStats,
  GroupWeeklyReport,
  GroupHealthFlag,
  MemberGroupComparison,
  ListGroupsResponse,
  GroupMembersResponse,
  GroupInvitesResponse,
  GroupStatsResponse,
  GroupPermissionsResponse,
} from '@pierre/shared-types';
import { ENDPOINTS } from '../core/endpoints';

// Re-export types for consumers
export type {
  CoachingGroup,
  CreateGroupRequest,
  UpdateGroupRequest,
  JoinGroupRequest,
  GroupSummary,
  GroupMember,
  GroupInvite,
  GroupAggregateStats,
  GroupWeeklyReport,
  GroupHealthFlag,
  MemberGroupComparison,
  ListGroupsResponse,
  GroupMembersResponse,
  GroupInvitesResponse,
  GroupStatsResponse,
  GroupPermissionsResponse,
};

/**
 * Creates the groups API methods bound to an axios instance.
 */
export function createGroupsApi(axios: AxiosInstance) {
  return {
    // ==================== GROUP CRUD ====================

    /** Create a new coaching group */
    async createGroup(request: CreateGroupRequest): Promise<CoachingGroup> {
      const response = await axios.post<CoachingGroup>(ENDPOINTS.GROUPS.LIST, request);
      return response.data;
    },

    /** List groups the current user belongs to */
    async listMyGroups(): Promise<ListGroupsResponse> {
      const response = await axios.get<ListGroupsResponse>(ENDPOINTS.GROUPS.LIST);
      return response.data;
    },

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

    /** Join a group via invite code */
    async joinGroup(request: JoinGroupRequest): Promise<GroupMember> {
      const response = await axios.post<GroupMember>(ENDPOINTS.GROUPS.JOIN, request);
      return response.data;
    },

    /** Leave a group */
    async leaveGroup(groupId: string): Promise<void> {
      await axios.post(ENDPOINTS.GROUPS.LEAVE(groupId));
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

    /** Get weekly report for a group */
    async getWeeklyReport(groupId: string): Promise<GroupWeeklyReport> {
      const response = await axios.get<GroupWeeklyReport>(ENDPOINTS.GROUPS.REPORT(groupId));
      return response.data;
    },

    /** Get health flags for a group */
    async getHealthFlags(groupId: string): Promise<GroupHealthFlag[]> {
      const response = await axios.get<GroupHealthFlag[]>(ENDPOINTS.GROUPS.HEALTH(groupId));
      return response.data;
    },

    /** Compare a member against group norms */
    async getMemberComparison(groupId: string, userId: string): Promise<MemberGroupComparison> {
      const response = await axios.get<MemberGroupComparison>(
        `${ENDPOINTS.GROUPS.MEMBERS(groupId)}/${userId}/comparison`,
      );
      return response.data;
    },
  };
}

export type GroupsApi = ReturnType<typeof createGroupsApi>;
