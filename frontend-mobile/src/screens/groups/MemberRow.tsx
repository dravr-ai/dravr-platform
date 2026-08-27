// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One member of a coaching group — avatar, display name, role badge, consent mark, admin controls
// ABOUTME: Drawn by Group info inside the group's own chat thread; the roles it offers match what the API allows

import React, { useMemo } from 'react';
import { View, Text, TouchableOpacity, ActivityIndicator } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { avatarSlot, initialsFor } from '@pierre/chat-utils';
import { useThemeColors } from '../../constants/theme';
import { InitialsAvatar } from '../../components/ui/InitialsAvatar';
import type { GroupMember, GroupRole } from '../../types';

/** What each role is called in the badge under a member's name. */
export const ROLE_LABELS: Record<GroupRole, string> = {
  owner: 'Owner',
  admin: 'Admin',
  member: 'Member',
};

/** What a member row says when the server has no display name for the row. */
const UNKNOWN_MEMBER = 'Unknown';

export interface MemberRowProps {
  member: GroupMember;
  /** The caller owns or administers the group: may remove plain members. */
  isAdmin: boolean;
  /** The caller owns the group: may promote and demote. */
  isOwner: boolean;
  onRemove: (member: GroupMember) => void;
  onChangeRole: (member: GroupMember, role: GroupRole) => void;
  isRemoving: boolean;
  isChangingRole: boolean;
}

export function MemberRow({
  member,
  isAdmin,
  isOwner,
  onRemove,
  onChangeRole,
  isRemoving,
  isChangingRole,
}: MemberRowProps) {
  const colors = useThemeColors();
  const roleColors = useMemo<Record<GroupRole, string>>(
    () => ({
      owner: colors.pierre.violet,
      admin: colors.pierre.activity,
      member: colors.pierre.recovery,
    }),
    [colors],
  );
  const roleColor = roleColors[member.role];
  const displayName = member.display_name ?? UNKNOWN_MEMBER;
  // The same initials and the same colour hash the conversation list uses, so
  // one person looks like one person wherever the app draws them.
  const slot = avatarSlot({ id: member.user_id, coach_id: null, group_id: null });

  return (
    <View className="flex-row items-center py-2.5" testID={`group-member-${member.user_id}`}>
      <InitialsAvatar initials={initialsFor(displayName)} slot={slot} />
      <View className="flex-1 ml-3">
        <Text className="text-base font-medium text-text-primary" numberOfLines={1}>
          {displayName}
        </Text>
        <View className="flex-row items-center mt-0.5">
          <View className="px-1.5 py-0.5 rounded" style={{ backgroundColor: `${roleColor}20` }}>
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
