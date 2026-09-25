// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mobile client's binding of the shared group hooks — its groups API and its read freshness
// ABOUTME: The hooks themselves live in @pierre/ui-logic, one definition for web and mobile

import { createGroupHooks, type GroupFreshness } from '@pierre/ui-logic';
import { groupsApi } from '../services/api';

/** The mobile client's freshness for the group reads the two clients set differently. */
const MOBILE_GROUP_FRESHNESS: GroupFreshness = {
  detail: 60_000,
  members: 60_000,
  stats: 5 * 60_000,
};

export const {
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
} = createGroupHooks(groupsApi, MOBILE_GROUP_FRESHNESS);
