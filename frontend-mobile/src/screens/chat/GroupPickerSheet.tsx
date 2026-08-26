// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Bottom sheet listing the athlete's coaching groups so a group chat can start from the chat "+"
// ABOUTME: Picking a group hands it back; the conversation itself is created by useStartGroupConversation

import React from 'react';
import { View, Text, TouchableOpacity, Modal, FlatList, ActivityIndicator } from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import { useRouter } from 'expo-router';
import { useThemeColors } from '../../constants/theme';
import { useMyGroups } from '../../hooks/useGroups';
import { GROUPS_ROUTE } from '../../navigation/routes';
import type { GroupSummary } from '../../types';

interface GroupPickerSheetProps {
  visible: boolean;
  onClose: () => void;
  /** The group the athlete chose; the caller opens its conversation. */
  onPick: (group: GroupSummary) => void;
  /** True while the chosen group's conversation is being created. */
  isStarting: boolean;
}

type GroupPickerBodyProps = Omit<GroupPickerSheetProps, 'visible'>;

/**
 * The list inside the sheet. Mounted only while the sheet is open, so the
 * groups query runs when the athlete asks for a group chat and not for every
 * screen that merely hosts the "+".
 */
function GroupPickerBody({ onClose, onPick, isStarting }: GroupPickerBodyProps) {
  const colors = useThemeColors();
  const router = useRouter();
  const { groups, isLoading } = useMyGroups();

  const goToGroups = () => {
    onClose();
    router.navigate(GROUPS_ROUTE);
  };

  if (isLoading || isStarting) {
    return <ActivityIndicator testID="group-picker-loading" color={colors.pierre.violet} />;
  }

  if (groups.length === 0) {
    return (
      <View className="items-center py-6" testID="group-picker-empty">
        <Text className="text-base text-text-secondary text-center mb-4">
          You are not in a coaching group yet. Join or create one to chat as a group.
        </Text>
        <TouchableOpacity
          className="px-4 py-2 rounded-lg"
          style={{ backgroundColor: colors.pierre.violet }}
          onPress={goToGroups}
          testID="group-picker-go-to-groups"
        >
          <Text className="text-sm font-medium" style={{ color: colors.tokens.onPrimary }}>
            Go to Groups
          </Text>
        </TouchableOpacity>
      </View>
    );
  }

  return (
    <FlatList
      data={groups}
      keyExtractor={(group) => group.id}
      testID="group-picker-list"
      renderItem={({ item }) => (
        <TouchableOpacity
          className="flex-row items-center py-3 border-b border-border-subtle"
          onPress={() => onPick(item)}
          accessibilityRole="button"
          accessibilityLabel={`Start a group chat with ${item.name}`}
          testID={`group-picker-option-${item.id}`}
        >
          <Ionicons name="people-outline" size={20} color={colors.pierre.violet} />
          <View className="flex-1 ml-3">
            <Text className="text-base text-text-primary" numberOfLines={1}>
              {item.name}
            </Text>
            <Text className="text-xs text-text-tertiary">
              {item.member_count} {item.member_count === 1 ? 'member' : 'members'}
            </Text>
          </View>
          <Text className="text-xl text-text-tertiary ml-2">›</Text>
        </TouchableOpacity>
      )}
    />
  );
}

/**
 * The second step of "New group chat": which room.
 *
 * The list is the same `useMyGroups` query the Groups tab reads, so a group
 * joined a moment ago is offered here without a second fetch path. An athlete
 * in no group is sent to the Groups tab to join or create one — the sheet
 * never offers a group the server would refuse.
 */
export function GroupPickerSheet({ visible, onClose, onPick, isStarting }: GroupPickerSheetProps) {
  const colors = useThemeColors();

  return (
    <Modal visible={visible} animationType="slide" transparent onRequestClose={onClose}>
      <View className="flex-1 justify-end bg-black/40">
        <View
          className="rounded-t-2xl px-4 pt-4 pb-8"
          style={{ backgroundColor: colors.background.secondary, maxHeight: '75%' }}
          testID="group-picker-sheet"
        >
          <View className="flex-row items-center justify-between mb-3">
            <Text className="text-lg font-semibold text-text-primary">New group chat</Text>
            <TouchableOpacity onPress={onClose} testID="group-picker-close" accessibilityLabel="Close">
              <Ionicons name="close" size={22} color={colors.text.secondary} />
            </TouchableOpacity>
          </View>

          {visible && <GroupPickerBody onClose={onClose} onPick={onPick} isStarting={isStarting} />}
        </View>
      </View>
    </Modal>
  );
}
