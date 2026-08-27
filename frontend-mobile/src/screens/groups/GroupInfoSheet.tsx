// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Group info for a group thread — members, invites, coach, settings, analytics, room, leave and delete
// ABOUTME: Everything the retired Groups tab held, re-homed where Telegram puts it: behind the chat header

import React, { useCallback, useMemo, useState } from 'react';
import { View, Text, TouchableOpacity, ActivityIndicator, Alert, ScrollView, Share, Switch } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { MENTION_PREFIX } from '@pierre/shared-constants';
import { useThemeColors } from '../../constants/theme';
import { CollapsibleSection, Input } from '../../components/ui';
import { useAuth } from '../../contexts/AuthContext';
import { useCoachInfo } from '../../hooks/useCoachInfo';
import {
  useCreateInvite,
  useDeactivateInvite,
  useDeleteGroup,
  useGroup,
  useGroupInvites,
  useGroupMembers,
  useGroupPermissions,
  useGroupStats,
  useLeaveGroup,
  useRemoveCoach,
  useRemoveMember,
  useUpdateGroup,
  useUpdateMemberRole,
  useUpdatePeerConsent,
} from '../../hooks/useGroups';
import { GroupInsightsSection } from './GroupInsightsSection';
import { GroupTranscriptSection } from './GroupTranscriptSection';
import { MemberRow } from './MemberRow';
import type { GroupMember, GroupRole } from '../../types';

/** How long an invite created from this sheet stays redeemable. */
const INVITE_LIFETIME_DAYS = 7;

/** Where a shared invite code sends someone; the web app re-homes it into chat. */
const INVITE_LINK_BASE = 'https://app.dravr.ai/groups/join';

export interface GroupInfoSheetProps {
  /** The group this thread is scoped to. */
  groupId: string;
  /** The thread's own title, shown until the group loads. */
  fallbackName: string | null;
  /** Close the host sheet. */
  onClose: () => void;
  /** The athlete is no longer in this group: go back to the conversation list. */
  onLeft: () => void;
}

function errorText(err: unknown, fallback: string): string {
  return err instanceof Error ? err.message : fallback;
}

/**
 * Everything a member or an admin can do about the group, from inside the
 * group's own chat.
 *
 * Sections collapse because a sheet over a transcript has little room and an
 * athlete usually opens it for one thing. The admin controls appear only for
 * an admin: the API refuses them from anyone else, so showing them would
 * advertise a 403.
 */
export function GroupInfoSheet({ groupId, fallbackName, onClose, onLeft }: GroupInfoSheetProps) {
  const colors = useThemeColors();
  const { user } = useAuth();

  const { group, isLoading: isLoadingGroup } = useGroup(groupId);
  const { members, isLoading: isLoadingMembers } = useGroupMembers(groupId);
  const { stats, isLoading: isLoadingStats } = useGroupStats(groupId);
  const { invites, isLoading: isLoadingInvites } = useGroupInvites(groupId);
  const { weeklyDigest } = useGroupPermissions();
  const { createInvite, isPending: isCreatingInvite } = useCreateInvite(groupId);
  const { deactivateInvite } = useDeactivateInvite(groupId);
  const { updateGroup, isPending: isUpdatingGroup } = useUpdateGroup(groupId);
  const { updateConsent, isPending: isSavingConsent } = useUpdatePeerConsent(groupId);
  const { updateRole } = useUpdateMemberRole(groupId);
  const { removeMember } = useRemoveMember(groupId);
  const { removeCoach } = useRemoveCoach(groupId);
  const { leaveGroup, isPending: isLeaving } = useLeaveGroup();
  const { deleteGroup, isPending: isDeleting } = useDeleteGroup();
  const { coach: aiCoach } = useCoachInfo(group?.coach_id ?? null);

  const [removingMemberId, setRemovingMemberId] = useState<string | null>(null);
  const [roleChangingUserId, setRoleChangingUserId] = useState<string | null>(null);
  const [nameDraft, setNameDraft] = useState<string | null>(null);
  const [descriptionDraft, setDescriptionDraft] = useState<string | null>(null);

  // The caller's own membership row, matched on their user id. Taking the
  // first active row instead handed whoever the server listed first the
  // caller's role badge and, worse, the caller's consent switch.
  const myMembership = useMemo(
    () => members.find((member) => member.user_id === user?.id),
    [members, user?.id],
  );
  const isAdmin = myMembership?.role === 'owner' || myMembership?.role === 'admin';
  const isOwner = myMembership?.role === 'owner';
  const activeInvites = useMemo(() => invites.filter((invite) => invite.is_active), [invites]);

  const handleRemoveMember = useCallback(
    (member: GroupMember) => {
      Alert.alert('Remove Member', `Remove ${member.display_name ?? 'this member'} from the group?`, [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Remove',
          style: 'destructive',
          onPress: async () => {
            try {
              setRemovingMemberId(member.user_id);
              await removeMember(member.user_id);
            } catch (err) {
              Alert.alert('Error', errorText(err, 'Failed to remove member'));
            } finally {
              setRemovingMemberId(null);
            }
          },
        },
      ]);
    },
    [removeMember],
  );

  const handleChangeRole = useCallback(
    async (member: GroupMember, role: GroupRole) => {
      try {
        setRoleChangingUserId(member.user_id);
        await updateRole({ userId: member.user_id, role });
      } catch (err) {
        Alert.alert('Error', errorText(err, 'Failed to update role'));
      } finally {
        setRoleChangingUserId(null);
      }
    },
    [updateRole],
  );

  const createInviteOfKind = useCallback(
    async (kind: 'member' | 'coach') => {
      try {
        const invite = await createInvite(
          kind === 'coach'
            ? { expires_in_days: INVITE_LIFETIME_DAYS, kind: 'coach' as const }
            : { expires_in_days: INVITE_LIFETIME_DAYS },
        );
        await Share.share({
          message:
            kind === 'coach'
              ? `Coach invite for ${group?.name ?? 'our group'}: ${INVITE_LINK_BASE}/${invite.code}`
              : `Join ${group?.name ?? 'our group'}: ${INVITE_LINK_BASE}/${invite.code}`,
        });
      } catch (err) {
        Alert.alert('Error', errorText(err, 'Failed to create invite'));
      }
    },
    [createInvite, group?.name],
  );

  const handleShareInvite = useCallback(() => {
    Alert.alert('Create Invite', 'Who is this invite for?', [
      { text: 'Member (athlete)', onPress: () => void createInviteOfKind('member') },
      { text: 'Coach', onPress: () => void createInviteOfKind('coach') },
      { text: 'Cancel', style: 'cancel' },
    ]);
  }, [createInviteOfKind]);

  const handleDeactivateInvite = useCallback(
    (inviteId: string, code: string) => {
      Alert.alert('Deactivate Invite', `Stop accepting code ${code}?`, [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Deactivate',
          style: 'destructive',
          onPress: async () => {
            try {
              await deactivateInvite(inviteId);
            } catch (err) {
              Alert.alert('Error', errorText(err, 'Failed to deactivate invite'));
            }
          },
        },
      ]);
    },
    [deactivateInvite],
  );

  const handleRemoveCoach = useCallback(() => {
    Alert.alert('Remove Coach', 'Detach the human coach from this group?', [
      { text: 'Cancel', style: 'cancel' },
      {
        text: 'Remove',
        style: 'destructive',
        onPress: async () => {
          try {
            await removeCoach();
          } catch (err) {
            Alert.alert('Error', errorText(err, 'Failed to remove coach'));
          }
        },
      },
    ]);
  }, [removeCoach]);

  const saveIdentity = useCallback(async () => {
    const name = (nameDraft ?? group?.name ?? '').trim();
    if (!name) return;
    try {
      await updateGroup({ name, description: (descriptionDraft ?? group?.description ?? '').trim() });
      setNameDraft(null);
      setDescriptionDraft(null);
    } catch (err) {
      Alert.alert('Error', errorText(err, 'Failed to update group'));
    }
  }, [nameDraft, descriptionDraft, group?.name, group?.description, updateGroup]);

  const setGroupFlag = useCallback(
    async (patch: { peer_data_sharing?: boolean; respond_mode?: 'all' | 'mentions' }) => {
      try {
        await updateGroup(patch);
      } catch (err) {
        Alert.alert('Error', errorText(err, 'Failed to update group'));
      }
    },
    [updateGroup],
  );

  const handleConsentChange = useCallback(
    async (consent: boolean) => {
      try {
        await updateConsent(consent);
      } catch (err) {
        Alert.alert('Error', errorText(err, 'Failed to update sharing consent'));
      }
    },
    [updateConsent],
  );

  const handleLeave = useCallback(() => {
    Alert.alert('Leave Group', `Leave "${group?.name ?? 'this group'}"?`, [
      { text: 'Cancel', style: 'cancel' },
      {
        text: 'Leave',
        style: 'destructive',
        onPress: async () => {
          try {
            await leaveGroup(groupId);
            onClose();
            onLeft();
          } catch (err) {
            Alert.alert('Error', errorText(err, 'Failed to leave group'));
          }
        },
      },
    ]);
  }, [group?.name, groupId, leaveGroup, onClose, onLeft]);

  const handleDelete = useCallback(() => {
    Alert.alert(
      'Archive Group',
      `Archive "${group?.name ?? 'this group'}"? Members lose access to its shared coaching.`,
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Archive',
          style: 'destructive',
          onPress: async () => {
            try {
              await deleteGroup(groupId);
              onClose();
              onLeft();
            } catch (err) {
              Alert.alert('Error', errorText(err, 'Failed to archive group'));
            }
          },
        },
      ],
    );
  }, [group?.name, groupId, deleteGroup, onClose, onLeft]);

  if (isLoadingGroup && !group) {
    return (
      <View className="py-10 items-center" testID="group-info-loading">
        <ActivityIndicator size="large" color={colors.pierre.violet} />
      </View>
    );
  }

  return (
    <ScrollView testID="group-info-sheet" keyboardShouldPersistTaps="handled">
      <Text className="text-lg font-bold text-text-primary" testID="group-info-name">
        {group?.name ?? fallbackName ?? 'Group'}
      </Text>
      {group?.description ? (
        <Text className="text-sm text-text-secondary mt-1" testID="group-info-description">
          {group.description}
        </Text>
      ) : null}

      <View className="mt-4">
        <CollapsibleSection
          title={`Members (${members.length})`}
          defaultExpanded
          testID="group-info-members"
        >
          {isLoadingMembers ? (
            <ActivityIndicator size="small" color={colors.pierre.violet} />
          ) : (
            members.map((member) => (
              <MemberRow
                key={member.id}
                member={member}
                isAdmin={isAdmin}
                isOwner={isOwner}
                onRemove={handleRemoveMember}
                onChangeRole={handleChangeRole}
                isRemoving={removingMemberId === member.user_id}
                isChangingRole={roleChangingUserId === member.user_id}
              />
            ))
          )}
        </CollapsibleSection>

        {isAdmin && (
          <CollapsibleSection title={`Invites (${activeInvites.length})`} testID="group-info-invites">
            <TouchableOpacity
              className="flex-row items-center py-2"
              onPress={handleShareInvite}
              disabled={isCreatingInvite}
              accessibilityRole="button"
              testID="group-info-create-invite"
            >
              {isCreatingInvite ? (
                <ActivityIndicator size="small" color={colors.pierre.violet} />
              ) : (
                <Feather name="share" size={18} color={colors.pierre.violet} />
              )}
              <Text className="text-base text-text-primary ml-3">Create and share an invite</Text>
            </TouchableOpacity>

            {isLoadingInvites ? (
              <ActivityIndicator size="small" color={colors.pierre.violet} />
            ) : activeInvites.length === 0 ? (
              <Text className="text-sm text-text-tertiary py-2" testID="group-invites-empty">
                No active invites.
              </Text>
            ) : (
              activeInvites.map((invite) => (
                <View
                  key={invite.id}
                  className="flex-row items-center py-2.5 border-b border-border-subtle"
                  testID={`group-invite-${invite.id}`}
                >
                  <View className="flex-1 pr-3">
                    <Text className="text-sm font-mono text-text-primary">{invite.code}</Text>
                    <Text className="text-xs text-text-tertiary mt-0.5">
                      {invite.kind === 'coach' ? 'Coach invite' : 'Member invite'} · used {invite.use_count}×
                    </Text>
                  </View>
                  <TouchableOpacity
                    className="px-3 py-1.5 rounded-md bg-error/20"
                    onPress={() => handleDeactivateInvite(invite.id, invite.code)}
                    testID={`deactivate-invite-${invite.id}`}
                  >
                    <Text className="text-error text-sm font-semibold">Deactivate</Text>
                  </TouchableOpacity>
                </View>
              ))
            )}
          </CollapsibleSection>
        )}

        <CollapsibleSection title="Coach" testID="group-info-coach">
          <View className="flex-row items-center py-2">
            <Feather name="cpu" size={16} color={colors.pierre.violet} />
            <Text className="text-sm text-text-primary ml-2 flex-1" testID="group-info-ai-coach">
              {aiCoach?.title ?? 'AI coach'}
              {aiCoach?.handle ? ` · ${MENTION_PREFIX}${aiCoach.handle}` : ''}
            </Text>
          </View>
          {group?.coach_user_id ? (
            <View className="flex-row items-center py-2" testID="group-info-human-coach">
              <Feather name="user-check" size={16} color={colors.pierre.violet} />
              <Text className="text-sm text-text-primary ml-2 flex-1">Human coach attached</Text>
              {isAdmin && (
                <TouchableOpacity onPress={handleRemoveCoach} testID="remove-coach-button">
                  <Text className="text-sm font-semibold text-text-secondary">Remove</Text>
                </TouchableOpacity>
              )}
            </View>
          ) : (
            <Text className="text-xs text-text-tertiary py-2">
              No human coach attached. Share a coach invite to bring one in.
            </Text>
          )}
        </CollapsibleSection>

        <CollapsibleSection title="Settings" testID="group-info-settings">
          {isAdmin && group && (
            <>
              <Input
                label="Name"
                value={nameDraft ?? group.name}
                onChangeText={setNameDraft}
                testID="group-name-input"
              />
              <Input
                label="Description"
                value={descriptionDraft ?? group.description ?? ''}
                onChangeText={setDescriptionDraft}
                testID="group-description-input"
              />
              <TouchableOpacity
                className="flex-row items-center justify-center py-2.5 rounded-xl mb-3"
                style={{ backgroundColor: colors.pierre.violet }}
                onPress={() => void saveIdentity()}
                disabled={isUpdatingGroup || (nameDraft === null && descriptionDraft === null)}
                accessibilityRole="button"
                testID="group-save-identity"
              >
                <Text className="text-sm font-semibold" style={{ color: colors.tokens.onPrimary }}>
                  Save
                </Text>
              </TouchableOpacity>

              <View className="flex-row items-center justify-between py-3 border-b border-border-subtle">
                <View className="flex-1 pr-3">
                  <Text className="text-sm font-semibold text-text-primary">Peer data sharing</Text>
                  <Text className="text-xs text-text-tertiary mt-1">
                    Allows consenting members to be compared with each other.
                  </Text>
                </View>
                <Switch
                  value={group.peer_data_sharing}
                  onValueChange={(value) => void setGroupFlag({ peer_data_sharing: value })}
                  disabled={isUpdatingGroup}
                  trackColor={{ false: colors.border.default, true: colors.pierre.violet }}
                  testID="group-peer-sharing-switch"
                />
              </View>

              <View className="flex-row items-center justify-between py-3 border-b border-border-subtle">
                <View className="flex-1 pr-3">
                  <Text className="text-sm font-semibold text-text-primary">Reply on mention only</Text>
                  <Text className="text-xs text-text-tertiary mt-1">
                    Off, the coach answers every message in the bound channel.
                  </Text>
                </View>
                <Switch
                  value={group.respond_mode === 'mentions'}
                  onValueChange={(value) => void setGroupFlag({ respond_mode: value ? 'mentions' : 'all' })}
                  disabled={isUpdatingGroup}
                  trackColor={{ false: colors.border.default, true: colors.pierre.violet }}
                  testID="group-respond-mode-switch"
                />
              </View>
            </>
          )}

          {/* The caller's own consent. The group can allow peer sharing, but
              each athlete still decides whether their own training data is
              part of it, and this is the only in-app place to say so. */}
          {myMembership && (
            <View className="flex-row items-center justify-between py-3" testID="peer-consent-card">
              <View className="flex-1 pr-3">
                <Text className="text-sm font-semibold text-text-primary">Share my training data</Text>
                <Text className="text-xs text-text-tertiary mt-1">
                  {group?.peer_data_sharing
                    ? 'Lets the coach compare you with the rest of the group.'
                    : 'Group sharing is off, so this stays private either way.'}
                </Text>
              </View>
              <Switch
                value={myMembership.peer_sharing_consent}
                onValueChange={(value) => void handleConsentChange(value)}
                disabled={isSavingConsent}
                trackColor={{ false: colors.border.default, true: colors.pierre.violet }}
                testID="peer-consent-switch"
              />
            </View>
          )}
        </CollapsibleSection>

        <CollapsibleSection title="Analytics" testID="group-info-analytics">
          <View className="flex-row py-2">
            <View className="flex-1 items-center">
              <Text className="text-lg font-bold text-text-primary" testID="group-stat-members">
                {members.length}
              </Text>
              <Text className="text-xs text-text-tertiary mt-0.5">Members</Text>
            </View>
            <View className="flex-1 items-center">
              <Text className="text-lg font-bold text-text-primary" testID="group-stat-active">
                {isLoadingStats ? '…' : String(stats?.active_members ?? members.length)}
              </Text>
              <Text className="text-xs text-text-tertiary mt-0.5">Active</Text>
            </View>
            <View className="flex-1 items-center">
              <Text className="text-lg font-bold text-text-primary" testID="group-stat-volume">
                {isLoadingStats
                  ? '…'
                  : stats?.avg_weekly_volume_km !== undefined
                    ? stats.avg_weekly_volume_km.toFixed(1)
                    : '--'}
              </Text>
              <Text className="text-xs text-text-tertiary mt-0.5">Avg Vol (km)</Text>
            </View>
          </View>
          <GroupInsightsSection groupId={groupId} isAdmin={isAdmin} weeklyDigestEnabled={weeklyDigest} />
        </CollapsibleSection>

        <CollapsibleSection title="Room" testID="group-info-room">
          <GroupTranscriptSection groupId={groupId} />
        </CollapsibleSection>
      </View>

      {!isOwner && (
        <TouchableOpacity
          className="flex-row items-center justify-center py-3 rounded-xl gap-2 border border-error/30 mt-2"
          onPress={handleLeave}
          disabled={isLeaving}
          accessibilityRole="button"
          testID="leave-group-button"
        >
          {isLeaving ? (
            <ActivityIndicator size="small" color={colors.error} />
          ) : (
            <>
              <Feather name="log-out" size={18} color={colors.error} />
              <Text className="text-base font-semibold text-error">Leave group</Text>
            </>
          )}
        </TouchableOpacity>
      )}

      {isOwner && (
        <TouchableOpacity
          className="flex-row items-center justify-center py-3 rounded-xl gap-2 border border-error/30 mt-2"
          onPress={handleDelete}
          disabled={isDeleting}
          accessibilityRole="button"
          testID="archive-group-button"
        >
          {isDeleting ? (
            <ActivityIndicator size="small" color={colors.error} />
          ) : (
            <>
              <Feather name="archive" size={18} color={colors.error} />
              <Text className="text-base font-semibold text-error">Archive group</Text>
            </>
          )}
        </TouchableOpacity>
      )}
    </ScrollView>
  );
}
