// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Reusable group card component for displaying coaching group info
// ABOUTME: Shows group name, member count, role badge, and navigates to detail on press

import React, { useMemo } from 'react';
import { View, Text, TouchableOpacity, type ViewStyle } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { glassCard, useThemeColors } from '../../constants/theme';
import type { GroupSummary, GroupRole } from '../../types';

const cardStyle: ViewStyle = {
  ...glassCard,
  borderRadius: 12,
  borderColor: 'rgba(139, 92, 246, 0.15)',
};

const ROLE_LABELS: Record<GroupRole, string> = {
  owner: 'Owner',
  admin: 'Admin',
  member: 'Member',
};

interface GroupCardProps {
  group: GroupSummary;
  onPress: (group: GroupSummary) => void;
  testID?: string;
}

export function GroupCard({ group, onPress, testID }: GroupCardProps) {
  const colors = useThemeColors();
  const roleColors = useMemo<Record<GroupRole, string>>(() => ({
    owner: colors.pierre.violet,
    admin: colors.pierre.activity,
    member: colors.pierre.recovery,
  }), [colors]);
  const roleColor = roleColors[group.my_role];
  const roleLabel = ROLE_LABELS[group.my_role];

  return (
    <TouchableOpacity
      className="p-4 mx-3 my-1.5 rounded-xl"
      style={cardStyle}
      onPress={() => onPress(group)}
      activeOpacity={0.7}
      testID={testID}
    >
      <View className="flex-row items-center justify-between mb-2">
        <Text
          className="text-lg font-semibold text-text-primary flex-1 mr-3"
          numberOfLines={1}
        >
          {group.name}
        </Text>
        <View
          className="px-2.5 py-1 rounded-full"
          style={{ backgroundColor: roleColor + '20' }}
        >
          <Text
            className="text-xs font-semibold"
            style={{ color: roleColor }}
          >
            {roleLabel}
          </Text>
        </View>
      </View>

      {group.description && (
        <Text
          className="text-sm text-text-secondary mb-3 leading-5"
          numberOfLines={2}
        >
          {group.description}
        </Text>
      )}

      <View className="flex-row items-center">
        <Feather name="users" size={14} color={colors.text.tertiary} />
        <Text className="text-sm text-text-tertiary ml-1.5">
          {group.member_count} {group.member_count === 1 ? 'member' : 'members'}
        </Text>

        {group.peer_data_sharing && (
          <>
            <View className="w-1 h-1 rounded-full bg-text-tertiary mx-2" />
            <Feather name="share-2" size={12} color={colors.text.tertiary} />
            <Text className="text-xs text-text-tertiary ml-1">Peer sharing</Text>
          </>
        )}

        {!group.is_active && (
          <>
            <View className="w-1 h-1 rounded-full bg-text-tertiary mx-2" />
            <Text className="text-xs text-warning">Archived</Text>
          </>
        )}
      </View>
    </TouchableOpacity>
  );
}
