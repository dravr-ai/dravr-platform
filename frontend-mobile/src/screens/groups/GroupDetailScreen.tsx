// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Group detail screen showing group info, stats, members, and actions
// ABOUTME: Supports admin actions for owners/admins and common actions for all members

import React, { useCallback, useState, useMemo } from 'react';
import {
  View,
  Text,
  Modal,
  ScrollView,
  Switch,
  TouchableOpacity,
  ActivityIndicator,
  RefreshControl,
  Alert,
  type ViewStyle,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useRouter, useLocalSearchParams } from 'expo-router';
import { Feather } from '@expo/vector-icons';
import type { ComponentProps } from 'react';
import { spacing, glassCard, buttonGlow, useThemeColors } from '../../constants/theme';
import {
  useGroup,
  useGroupMembers,
  useGroupStats,
  useGroupActions,
  useGroupInvites,
  useGroupPermissions,
  useUpdateGroup,
  useDeleteGroup,
  useDeactivateInvite,
  useUpdateMemberRole,
  useUpdatePeerConsent,
} from '../../hooks/useGroups';
import { useAuth } from '../../contexts/AuthContext';
import { chatApi, groupsApi } from '../../services/api';
import { GroupInsightsSection } from './GroupInsightsSection';
import { GroupTranscriptSection } from './GroupTranscriptSection';
import type { GroupMember, GroupRole } from '../../types';

type FeatherIconName = ComponentProps<typeof Feather>['name'];

const sectionCardStyle: ViewStyle = {
  borderRadius: 12,
  ...glassCard,
};

const ROLE_LABELS: Record<GroupRole, string> = {
  owner: 'Owner',
  admin: 'Admin',
  member: 'Member',
};

interface StatItemProps {
  icon: FeatherIconName;
  label: string;
  value: string;
}

function StatItem({ icon, label, value }: StatItemProps) {
  const colors = useThemeColors();
  return (
    <View className="items-center flex-1">
      <Feather name={icon} size={18} color={colors.pierre.violet} />
      <Text className="text-text-primary text-lg font-bold mt-1">{value}</Text>
      <Text className="text-text-tertiary text-xs mt-0.5">{label}</Text>
    </View>
  );
}

interface MemberRowProps {
  member: GroupMember;
  isAdmin: boolean;
  isOwner: boolean;
  onRemove: (member: GroupMember) => void;
  onChangeRole: (member: GroupMember, role: GroupRole) => void;
  isRemoving: boolean;
  isChangingRole: boolean;
}

function MemberRow({
  member,
  isAdmin,
  isOwner,
  onRemove,
  onChangeRole,
  isRemoving,
  isChangingRole,
}: MemberRowProps) {
  const colors = useThemeColors();
  const roleColors = useMemo<Record<GroupRole, string>>(() => ({
    owner: colors.pierre.violet,
    admin: colors.pierre.activity,
    member: colors.pierre.recovery,
  }), [colors]);
  const roleColor = roleColors[member.role];
  const displayName = member.display_name ?? 'Unknown';

  // Generate initials from display name
  const initials = displayName
    .split(' ')
    .map((part) => part[0])
    .slice(0, 2)
    .join('')
    .toUpperCase();

  // Generate consistent color from user_id
  const hash = member.user_id.split('').reduce((acc, char) => {
    return char.charCodeAt(0) + ((acc << 5) - acc);
  }, 0);
  const hue = Math.abs(hash % 360);
  const avatarColor = `hsl(${hue}, 70%, 50%)`;

  return (
    <View className="flex-row items-center py-2.5">
      <View
        className="w-10 h-10 rounded-full justify-center items-center"
        style={{ backgroundColor: avatarColor }}
      >
        <Text className="text-sm font-semibold text-text-primary">{initials}</Text>
      </View>
      <View className="flex-1 ml-3">
        <Text className="text-base font-medium text-text-primary" numberOfLines={1}>
          {displayName}
        </Text>
        <View className="flex-row items-center mt-0.5">
          <View
            className="px-1.5 py-0.5 rounded"
            style={{ backgroundColor: roleColor + '20' }}
          >
            <Text className="text-[10px] font-semibold" style={{ color: roleColor }}>
              {ROLE_LABELS[member.role]}
            </Text>
          </View>
          {member.peer_sharing_consent && (
            <View className="flex-row items-center ml-2">
              <Feather name="eye" size={10} color={colors.text.tertiary} />
              <Text className="text-[10px] text-text-tertiary ml-0.5">Sharing</Text>
            </View>
          )}
        </View>
      </View>
      {/* Promotion is the owner's call — the API rejects it from anyone else,
          so showing it to a plain admin would advertise a 403. */}
      {isOwner && member.role !== 'owner' && (
        <TouchableOpacity
          className="px-2 py-1 mr-1 rounded border border-border-strong"
          onPress={() => onChangeRole(member, member.role === 'admin' ? 'member' : 'admin')}
          disabled={isChangingRole}
          testID={`member-role-${member.user_id}`}
        >
          {isChangingRole ? (
            <ActivityIndicator size="small" color={colors.text.secondary} />
          ) : (
            <Text className="text-xs font-semibold text-text-secondary">
              {member.role === 'admin' ? 'Demote' : 'Promote'}
            </Text>
          )}
        </TouchableOpacity>
      )}
      {isAdmin && member.role === 'member' && (
        <TouchableOpacity
          className="p-2"
          onPress={() => onRemove(member)}
          disabled={isRemoving}
          testID={`member-remove-${member.user_id}`}
        >
          {isRemoving ? (
            <ActivityIndicator size="small" color={colors.text.secondary} />
          ) : (
            <Feather name="user-minus" size={18} color={colors.text.secondary} />
          )}
        </TouchableOpacity>
      )}
    </View>
  );
}

export function GroupDetailScreen() {
  const colors = useThemeColors();
  const router = useRouter();
  const { groupId } = useLocalSearchParams<{ groupId: string }>();
  const { user } = useAuth();
  const [removingMemberId, setRemovingMemberId] = useState<string | null>(null);
  const [roleChangingUserId, setRoleChangingUserId] = useState<string | null>(null);
  const [showAdminSheet, setShowAdminSheet] = useState(false);

  const { group, isLoading: isLoadingGroup, refetch: refetchGroup } = useGroup(groupId ?? '');
  const { members, isLoading: isLoadingMembers, refetch: refetchMembers } = useGroupMembers(groupId ?? '');
  const { stats, isLoading: isLoadingStats } = useGroupStats(groupId ?? '');
  const { leaveGroup, isLeaving } = useGroupActions();
  const { invites, isLoading: isLoadingInvites } = useGroupInvites(groupId ?? '');
  const { updateGroup, isPending: isUpdatingGroup } = useUpdateGroup(groupId ?? '');
  const { deleteGroup, isPending: isDeletingGroup } = useDeleteGroup();
  const { deactivateInvite } = useDeactivateInvite(groupId ?? '');
  const { updateRole } = useUpdateMemberRole(groupId ?? '');
  const { updateConsent, isPending: isSavingConsent } = useUpdatePeerConsent(groupId ?? '');
  const { weeklyDigest } = useGroupPermissions();
  const [isStartingChat, setIsStartingChat] = useState(false);

  const isLoading = isLoadingGroup || isLoadingMembers;
  const [isRefreshing, setIsRefreshing] = useState(false);

  // The endpoint returns only members who have not left, so every row here is
  // an active membership. Filtering on `left_at` client-side drops all of them:
  // MemberResponse does not serialize the field, so `undefined === null` is
  // false for every row -- which is how a group owner lost their own controls.
  const activeMembers = members;

  // The caller's own membership row, matched on their user id. Taking the
  // first active row instead handed whoever the server listed first the
  // caller's role badge and, worse, the caller's consent switch.
  const myMembership = useMemo(
    () => activeMembers.find((m) => m.user_id === user?.id),
    [activeMembers, user?.id],
  );
  const isAdmin = myMembership?.role === 'owner' || myMembership?.role === 'admin';
  const isOwner = myMembership?.role === 'owner';

  const handleRefresh = useCallback(async () => {
    setIsRefreshing(true);
    await Promise.all([refetchGroup(), refetchMembers()]);
    setIsRefreshing(false);
  }, [refetchGroup, refetchMembers]);

  const handleLeaveGroup = useCallback(() => {
    if (!groupId) return;
    Alert.alert(
      'Leave Group',
      `Are you sure you want to leave "${group?.name ?? 'this group'}"?`,
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Leave',
          style: 'destructive',
          onPress: async () => {
            try {
              await leaveGroup(groupId);
              router.back();
            } catch (err) {
              const msg = err instanceof Error ? err.message : 'Failed to leave group';
              Alert.alert('Error', msg);
            }
          },
        },
      ],
    );
  }, [groupId, group?.name, leaveGroup, router]);

  const handleRemoveMember = useCallback(
    async (member: GroupMember) => {
      if (!groupId) return;
      Alert.alert(
        'Remove Member',
        `Remove ${member.display_name ?? 'this member'} from the group?`,
        [
          { text: 'Cancel', style: 'cancel' },
          {
            text: 'Remove',
            style: 'destructive',
            onPress: async () => {
              try {
                setRemovingMemberId(member.user_id);
                await groupsApi.removeMember(groupId, member.user_id);
                await refetchMembers();
              } catch (err) {
                const msg = err instanceof Error ? err.message : 'Failed to remove member';
                Alert.alert('Error', msg);
              } finally {
                setRemovingMemberId(null);
              }
            },
          },
        ],
      );
    },
    [groupId, refetchMembers],
  );

  const handleChangeRole = useCallback(
    async (member: GroupMember, role: GroupRole) => {
      if (!groupId) return;
      try {
        setRoleChangingUserId(member.user_id);
        await updateRole({ userId: member.user_id, role });
        await refetchMembers();
      } catch (err) {
        const msg = err instanceof Error ? err.message : 'Failed to update role';
        Alert.alert('Error', msg);
      } finally {
        setRoleChangingUserId(null);
      }
    },
    [groupId, updateRole, refetchMembers],
  );

  /**
   * Set the caller's own peer-sharing consent for this group.
   *
   * Until now the only surface that could set it was the messaging bot, so an
   * athlete who joined from the app had no way to say yes or no to their
   * training data being read by the rest of the group.
   */
  const handleConsentChange = useCallback(
    async (consent: boolean) => {
      if (!groupId) return;
      try {
        await updateConsent(consent);
        await refetchMembers();
      } catch (err) {
        const msg = err instanceof Error ? err.message : 'Failed to update sharing consent';
        Alert.alert('Error', msg);
      }
    },
    [groupId, updateConsent, refetchMembers],
  );

  const handleToggleGroupSharing = useCallback(
    async (peerDataSharing: boolean) => {
      if (!groupId) return;
      try {
        await updateGroup({ peer_data_sharing: peerDataSharing });
        await refetchGroup();
      } catch (err) {
        const msg = err instanceof Error ? err.message : 'Failed to update group';
        Alert.alert('Error', msg);
      }
    },
    [groupId, updateGroup, refetchGroup],
  );

  const handleRespondModeChange = useCallback(
    async (mentionsOnly: boolean) => {
      if (!groupId) return;
      try {
        await updateGroup({ respond_mode: mentionsOnly ? 'mentions' : 'all' });
        await refetchGroup();
      } catch (err) {
        const msg = err instanceof Error ? err.message : 'Failed to update group';
        Alert.alert('Error', msg);
      }
    },
    [groupId, updateGroup, refetchGroup],
  );

  const handleDeactivateInvite = useCallback(
    async (inviteId: string, code: string) => {
      Alert.alert('Deactivate Invite', `Stop accepting code ${code}?`, [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Deactivate',
          style: 'destructive',
          onPress: async () => {
            try {
              await deactivateInvite(inviteId);
            } catch (err) {
              const msg = err instanceof Error ? err.message : 'Failed to deactivate invite';
              Alert.alert('Error', msg);
            }
          },
        },
      ]);
    },
    [deactivateInvite],
  );

  const handleDeleteGroup = useCallback(() => {
    if (!groupId) return;
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
              setShowAdminSheet(false);
              router.back();
            } catch (err) {
              const msg = err instanceof Error ? err.message : 'Failed to archive group';
              Alert.alert('Error', msg);
            }
          },
        },
      ],
    );
  }, [groupId, group?.name, deleteGroup, router]);

  const createInviteOfKind = useCallback(
    async (kind: 'member' | 'coach') => {
      if (!groupId) return;
      try {
        const request =
          kind === 'coach'
            ? { expires_in_days: 7, kind: 'coach' as const }
            : { expires_in_days: 7 };
        const invite = await groupsApi.createInvite(groupId, request);
        Alert.alert(
          kind === 'coach' ? 'Coach Invite Code' : 'Invite Code',
          kind === 'coach'
            ? `${invite.code}\n\nShare with the coach who will oversee this group.`
            : invite.code,
          [{ text: 'OK' }],
        );
      } catch (err) {
        const msg = err instanceof Error ? err.message : 'Failed to create invite';
        Alert.alert('Error', msg);
      }
    },
    [groupId],
  );

  const handleShareInvite = useCallback(() => {
    Alert.alert('Create Invite', 'Who is this invite for?', [
      { text: 'Member (athlete)', onPress: () => void createInviteOfKind('member') },
      { text: 'Coach', onPress: () => void createInviteOfKind('coach') },
      { text: 'Cancel', style: 'cancel' },
    ]);
  }, [createInviteOfKind]);

  /**
   * Open a chat scoped to this group.
   *
   * The button used to push the chat tab a `coachId` param that nothing on
   * that screen reads, so the athlete landed on the generic coach picker with
   * no sign of which room they came from. The conversation is created here
   * instead, carrying `group_id` — the field that turns on group context and
   * the peer-grounding stage server-side — and titled with the group's name so
   * the chat header names the room.
   */
  const handleOpenGroupChat = useCallback(async () => {
    if (!groupId || !group) return;
    try {
      setIsStartingChat(true);
      const conversation = await chatApi.createConversation({
        title: group.name,
        coach_id: group.coach_id,
        group_id: groupId,
      });
      router.push({
        pathname: '/(app)/(tabs)/(chat)',
        params: { conversationId: conversation.id },
      });
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to open the group chat';
      Alert.alert('Error', msg);
    } finally {
      setIsStartingChat(false);
    }
  }, [groupId, group, router]);

  const handleRemoveCoach = useCallback(() => {
    if (!groupId) return;
    Alert.alert('Remove Coach', 'Detach the human coach from this group?', [
      { text: 'Cancel', style: 'cancel' },
      {
        text: 'Remove',
        style: 'destructive',
        onPress: async () => {
          try {
            await groupsApi.removeCoach(groupId);
            await refetchGroup();
          } catch (err) {
            const msg = err instanceof Error ? err.message : 'Failed to remove coach';
            Alert.alert('Error', msg);
          }
        },
      },
    ]);
  }, [groupId, refetchGroup]);

  if (isLoading && !group) {
    return (
      <SafeAreaView className="flex-1 bg-background-primary" testID="group-detail-screen">
        <View className="flex-1 justify-center items-center">
          <ActivityIndicator size="large" color={colors.pierre.violet} />
          <Text className="text-text-secondary mt-4">Loading group...</Text>
        </View>
      </SafeAreaView>
    );
  }

  if (!group) {
    return (
      <SafeAreaView className="flex-1 bg-background-primary" testID="group-detail-screen">
        <View className="flex-1 justify-center items-center p-6">
          <Feather name="alert-circle" size={48} color={colors.text.secondary} />
          <Text className="text-text-primary text-lg font-bold mt-4">Group Not Found</Text>
          <TouchableOpacity className="mt-4" onPress={() => router.back()}>
            <Text className="text-primary text-base font-semibold">Go Back</Text>
          </TouchableOpacity>
        </View>
      </SafeAreaView>
    );
  }

  return (
    <SafeAreaView className="flex-1 bg-background-primary" testID="group-detail-screen">
      {/* Header */}
      <View className="flex-row items-center px-4 py-3 border-b border-border-subtle">
        <TouchableOpacity className="p-2" onPress={() => router.back()} testID="back-button">
          <Feather name="arrow-left" size={22} color={colors.text.primary} />
        </TouchableOpacity>
        <Text className="flex-1 text-lg font-bold text-text-primary text-center" numberOfLines={1}>
          {group.name}
        </Text>
        {isAdmin ? (
          <TouchableOpacity
            className="p-2"
            onPress={() => setShowAdminSheet(true)}
            testID="group-admin-button"
          >
            <Feather name="settings" size={20} color={colors.text.primary} />
          </TouchableOpacity>
        ) : (
          <View className="w-10" />
        )}
      </View>

      <ScrollView
        className="flex-1"
        contentContainerStyle={{ padding: spacing.md, paddingBottom: 32 }}
        refreshControl={
          <RefreshControl
            refreshing={isRefreshing}
            onRefresh={handleRefresh}
            tintColor={colors.pierre.violet}
          />
        }
      >
        {/* Description */}
        {group.description && (
          <Text className="text-text-secondary text-base leading-6 mb-4">
            {group.description}
          </Text>
        )}

        {/* Quick Stats */}
        <View className="p-4 mb-4" style={sectionCardStyle}>
          <View className="flex-row">
            <StatItem
              icon="users"
              label="Members"
              value={String(activeMembers.length)}
            />
            <StatItem
              icon="activity"
              label="Active"
              value={
                isLoadingStats
                  ? '...'
                  : String(stats?.active_members ?? activeMembers.length)
              }
            />
            <StatItem
              icon="trending-up"
              label="Avg Vol (km)"
              value={
                isLoadingStats
                  ? '...'
                  : stats?.avg_weekly_volume_km !== undefined
                    ? stats.avg_weekly_volume_km.toFixed(1)
                    : '--'
              }
            />
          </View>
        </View>

        {/* Human coach */}
        {group.coach_user_id && (
          <View className="p-4 mb-4 flex-row items-center justify-between" style={sectionCardStyle}>
            <View className="flex-row items-center gap-2 flex-1">
              <Feather name="user-check" size={18} color={colors.pierre.violet} />
              <Text className="text-text-primary text-sm font-semibold">Human coach attached</Text>
            </View>
            {isAdmin && (
              <TouchableOpacity onPress={handleRemoveCoach} testID="remove-coach-button">
                <Text className="text-sm font-semibold" style={{ color: colors.text.secondary }}>
                  Remove
                </Text>
              </TouchableOpacity>
            )}
          </View>
        )}

        {/* Peer-data consent — the caller's own row. The group can allow peer
            sharing, but each athlete still decides whether their own training
            data is part of it, and this is the only in-app place to say so. */}
        {myMembership && (
          <View
            className="p-4 mb-4 flex-row items-center justify-between"
            style={sectionCardStyle}
            testID="peer-consent-card"
          >
            <View className="flex-1 pr-3">
              <Text className="text-text-primary text-sm font-semibold">
                Share my training data
              </Text>
              <Text className="text-text-tertiary text-xs mt-1">
                {group.peer_data_sharing
                  ? 'Lets the coach compare you with the rest of the group.'
                  : 'Group sharing is off, so this stays private either way.'}
              </Text>
            </View>
            <Switch
              value={myMembership.peer_sharing_consent}
              onValueChange={handleConsentChange}
              disabled={isSavingConsent}
              trackColor={{ false: colors.border.default, true: colors.pierre.violet }}
              testID="peer-consent-switch"
            />
          </View>
        )}

        {/* Action Buttons */}
        <View className="flex-row gap-3 mb-4">
          <TouchableOpacity
            className="flex-1 flex-row items-center justify-center py-3 rounded-xl gap-2"
            style={{
              backgroundColor: colors.pierre.violet,
              ...buttonGlow,
            }}
            onPress={() => void handleOpenGroupChat()}
            disabled={isStartingChat}
            testID="chat-with-coach-button"
          >
            {isStartingChat ? (
              <ActivityIndicator size="small" color={colors.tokens.onPrimary} />
            ) : (
              <>
                <Feather name="message-circle" size={18} color={colors.tokens.onPrimary} />
                <Text className="text-sm font-semibold" style={{ color: colors.tokens.onPrimary }}>Chat with Coach</Text>
              </>
            )}
          </TouchableOpacity>

          {isAdmin && (
            <TouchableOpacity
              className="flex-row items-center justify-center py-3 px-4 rounded-xl gap-2 border border-border-strong"
              onPress={handleShareInvite}
              testID="share-invite-button"
            >
              <Feather name="share" size={18} color={colors.text.primary} />
              <Text className="text-text-primary text-sm font-semibold">Invite</Text>
            </TouchableOpacity>
          )}
        </View>

        {/* Members Section */}
        <View className="p-4 mb-4" style={sectionCardStyle}>
          <View className="flex-row items-center justify-between mb-3">
            <Text className="text-text-primary text-base font-bold">
              Members ({activeMembers.length})
            </Text>
          </View>

          {isLoadingMembers ? (
            <ActivityIndicator size="small" color={colors.pierre.violet} />
          ) : (
            activeMembers.map((member) => (
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
        </View>

        {/* The shared room: one transcript across chat, web and messaging. */}
        <GroupTranscriptSection groupId={groupId ?? ''} />

        {/* Weekly report + health flags — computed on every request and,
            until now, rendered nowhere outside the digest scheduler. */}
        <GroupInsightsSection
          groupId={groupId ?? ''}
          isAdmin={isAdmin}
          weeklyDigestEnabled={weeklyDigest}
        />

        {/* Leave / Settings */}
        <View className="gap-3">
          {!isOwner && (
            <TouchableOpacity
              className="flex-row items-center justify-center py-3 rounded-xl gap-2 border border-error/30"
              onPress={handleLeaveGroup}
              disabled={isLeaving}
              testID="leave-group-button"
            >
              {isLeaving ? (
                <ActivityIndicator size="small" color={colors.error} />
              ) : (
                <>
                  <Feather name="log-out" size={18} color={colors.error} />
                  <Text className="text-error text-base font-semibold">Leave Group</Text>
                </>
              )}
            </TouchableOpacity>
          )}
        </View>
      </ScrollView>

      {/* Admin sheet — group settings, live invites and the archive control.
          Web wraps these three surfaces in hooks; mobile could reach none of
          them, so an owner had to open the web app to change anything. */}
      <Modal
        visible={showAdminSheet}
        animationType="slide"
        transparent
        onRequestClose={() => setShowAdminSheet(false)}
      >
        <View className="flex-1 bg-black/60 justify-end">
          <View
            className="bg-background-primary rounded-t-2xl max-h-[85%]"
            style={{ padding: spacing.md }}
            testID="group-admin-sheet"
          >
            <View className="flex-row items-center justify-between mb-3">
              <Text className="text-lg font-bold text-text-primary">Group Settings</Text>
              <TouchableOpacity
                className="p-2 -mr-2"
                onPress={() => setShowAdminSheet(false)}
                testID="close-group-admin-sheet"
              >
                <Feather name="x" size={22} color={colors.text.secondary} />
              </TouchableOpacity>
            </View>

            <ScrollView>
              <View className="flex-row items-center justify-between py-3 border-b border-border-subtle">
                <View className="flex-1 pr-3">
                  <Text className="text-text-primary text-sm font-semibold">Peer data sharing</Text>
                  <Text className="text-text-tertiary text-xs mt-1">
                    Allows consenting members to be compared with each other.
                  </Text>
                </View>
                <Switch
                  value={group.peer_data_sharing}
                  onValueChange={handleToggleGroupSharing}
                  disabled={isUpdatingGroup}
                  trackColor={{ false: colors.border.default, true: colors.pierre.violet }}
                  testID="group-peer-sharing-switch"
                />
              </View>

              <View className="flex-row items-center justify-between py-3 border-b border-border-subtle">
                <View className="flex-1 pr-3">
                  <Text className="text-text-primary text-sm font-semibold">Reply on mention only</Text>
                  <Text className="text-text-tertiary text-xs mt-1">
                    Off, the coach answers every message in the bound channel.
                  </Text>
                </View>
                <Switch
                  value={group.respond_mode === 'mentions'}
                  onValueChange={handleRespondModeChange}
                  disabled={isUpdatingGroup}
                  trackColor={{ false: colors.border.default, true: colors.pierre.violet }}
                  testID="group-respond-mode-switch"
                />
              </View>

              <Text className="text-text-primary text-base font-bold mt-4 mb-1">
                Active Invites ({invites.filter((invite) => invite.is_active).length})
              </Text>
              {isLoadingInvites ? (
                <ActivityIndicator size="small" color={colors.pierre.violet} />
              ) : invites.filter((invite) => invite.is_active).length === 0 ? (
                <Text className="text-text-tertiary text-sm py-2" testID="group-invites-empty">
                  No active invites.
                </Text>
              ) : (
                invites
                  .filter((invite) => invite.is_active)
                  .map((invite) => (
                    <View
                      key={invite.id}
                      className="flex-row items-center py-2.5 border-b border-border-subtle"
                      testID={`group-invite-${invite.id}`}
                    >
                      <View className="flex-1 pr-3">
                        <Text className="text-text-primary text-sm font-mono">{invite.code}</Text>
                        <Text className="text-text-tertiary text-xs mt-0.5">
                          {invite.kind === 'coach' ? 'Coach invite' : 'Member invite'} · used{' '}
                          {invite.use_count}×
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

              {isOwner && (
                <TouchableOpacity
                  className="flex-row items-center justify-center py-3 mt-5 rounded-xl gap-2 border border-error/30"
                  onPress={handleDeleteGroup}
                  disabled={isDeletingGroup}
                  testID="archive-group-button"
                >
                  {isDeletingGroup ? (
                    <ActivityIndicator size="small" color={colors.error} />
                  ) : (
                    <>
                      <Feather name="archive" size={18} color={colors.error} />
                      <Text className="text-error text-base font-semibold">Archive Group</Text>
                    </>
                  )}
                </TouchableOpacity>
              )}
            </ScrollView>
          </View>
        </View>
      </Modal>
    </SafeAreaView>
  );
}
