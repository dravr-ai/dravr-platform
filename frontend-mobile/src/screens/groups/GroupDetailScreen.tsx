// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Group detail screen showing group info, stats, members, and actions
// ABOUTME: Supports admin actions for owners/admins and common actions for all members

import React, { useCallback, useState } from 'react';
import {
  View,
  Text,
  ScrollView,
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
import { colors, spacing, glassCard, buttonGlow } from '../../constants/theme';
import { useGroup, useGroupMembers, useGroupStats, useGroupActions } from '../../hooks/useGroups';
import { groupsApi } from '../../services/api';
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

const ROLE_COLORS: Record<GroupRole, string> = {
  owner: colors.pierre.violet,
  admin: colors.pierre.activity,
  member: colors.pierre.recovery,
};

interface StatItemProps {
  icon: FeatherIconName;
  label: string;
  value: string;
}

function StatItem({ icon, label, value }: StatItemProps) {
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
  onRemove: (member: GroupMember) => void;
  isRemoving: boolean;
}

function MemberRow({ member, isAdmin, onRemove, isRemoving }: MemberRowProps) {
  const roleColor = ROLE_COLORS[member.role];
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
      {isAdmin && member.role === 'member' && (
        <TouchableOpacity
          className="p-2"
          onPress={() => onRemove(member)}
          disabled={isRemoving}
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
  const router = useRouter();
  const { groupId } = useLocalSearchParams<{ groupId: string }>();
  const [removingMemberId, setRemovingMemberId] = useState<string | null>(null);

  const { group, isLoading: isLoadingGroup, refetch: refetchGroup } = useGroup(groupId ?? '');
  const { members, isLoading: isLoadingMembers, refetch: refetchMembers } = useGroupMembers(groupId ?? '');
  const { stats, isLoading: isLoadingStats } = useGroupStats(groupId ?? '');
  const { leaveGroup, isLeaving } = useGroupActions();

  const isLoading = isLoadingGroup || isLoadingMembers;
  const [isRefreshing, setIsRefreshing] = useState(false);

  // Determine current user's role from the member list
  const myMembership = members.find((m) => m.left_at === null);
  const isAdmin = myMembership?.role === 'owner' || myMembership?.role === 'admin';
  const isOwner = myMembership?.role === 'owner';
  const activeMembers = members.filter((m) => m.left_at === null);

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

  const handleShareInvite = useCallback(async () => {
    if (!groupId) return;
    try {
      const invite = await groupsApi.createInvite(groupId, { expires_in_days: 7 });
      Alert.alert('Invite Code', invite.code, [{ text: 'OK' }]);
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to create invite';
      Alert.alert('Error', msg);
    }
  }, [groupId]);

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
            <Text className="text-pierre-violet text-base font-semibold">Go Back</Text>
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
        <Text className="flex-1 text-lg font-bold text-text-primary text-center mr-10" numberOfLines={1}>
          {group.name}
        </Text>
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

        {/* Action Buttons */}
        <View className="flex-row gap-3 mb-4">
          <TouchableOpacity
            className="flex-1 flex-row items-center justify-center py-3 rounded-xl gap-2"
            style={{
              backgroundColor: colors.pierre.violet,
              ...buttonGlow,
            }}
            onPress={() =>
              router.push({
                pathname: '/(app)/(tabs)/(chat)',
                params: { coachId: group.coach_id },
              })
            }
            testID="chat-with-coach-button"
          >
            <Feather name="message-circle" size={18} color="#FFFFFF" />
            <Text className="text-on-surface text-sm font-semibold">Chat with Coach</Text>
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
                onRemove={handleRemoveMember}
                isRemoving={removingMemberId === member.user_id}
              />
            ))
          )}
        </View>

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
    </SafeAreaView>
  );
}
