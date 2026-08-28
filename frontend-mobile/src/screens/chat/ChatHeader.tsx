// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Chat header — back, the thread's initials avatar, its title, appearance, bell and the "+"
// ABOUTME: Tapping the avatar or the title opens the thread's info sheet, the way every messaging app does it

import React from 'react';
import { useTranslation } from '@pierre/i18n';
import { View, Text, TouchableOpacity } from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import { avatarSlot, deriveKind, initialsFor, UNTITLED_CONVERSATION } from '@pierre/chat-utils';
import { MENTION_PREFIX } from '@pierre/shared-constants';
import { spacing, useThemeColors } from '../../constants/theme';
import type { Conversation } from '../../types';
import { NotificationBellButton } from '../../components/notifications/NotificationBellButton';
import { AppearanceToggleButton } from '../../components/ui/AppearanceToggleButton';
import { InitialsAvatar } from '../../components/ui/InitialsAvatar';

/** What the header shows before the athlete has started a thread. */
export const NEW_CHAT_TITLE = 'New Chat';

interface ChatHeaderProps {
  currentConversation: Conversation | null;
  insetTop: number;
  /** Back to the conversation list the thread was opened from. */
  onBackPress: () => void;
  /** The chat "+": new chat, new group chat, add someone to this discussion. */
  onPlusPress: () => void;
  /** Open the thread's info sheet — group info, coach info, or the plain rows. */
  onTitlePress: () => void;
}

export function ChatHeader({
  currentConversation,
  insetTop,
  onBackPress,
  onPlusPress,
  onTitlePress,
}: ChatHeaderProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const title = currentConversation?.title?.trim() || (currentConversation ? UNTITLED_CONVERSATION : NEW_CHAT_TITLE);
  const kind = currentConversation ? deriveKind(currentConversation) : null;
  const handle = currentConversation?.coach_handle ?? null;

  return (
    <View
      className="flex-row items-center px-4 py-2 border-b border-border-subtle"
      style={{ paddingTop: insetTop + spacing.sm }}
    >
      {/* Back to the conversation list */}
      <TouchableOpacity
        className="w-10 h-10 items-center justify-center"
        onPress={onBackPress}
        accessibilityRole="button"
        accessibilityLabel={t('app.headerBackToChatsAria')}
        testID="back-button"
      >
        <Ionicons name="chevron-back" size={26} color={colors.text.primary} />
      </TouchableOpacity>

      <TouchableOpacity
        className={`flex-1 flex-row items-center ${currentConversation ? '' : 'justify-center'} mx-2`}
        onPress={onTitlePress}
        disabled={!currentConversation}
        accessibilityRole="button"
        accessibilityLabel={currentConversation ? `${title}, open chat info` : title}
        testID="chat-title-button"
      >
        {currentConversation && (
          <View className="mr-2">
            <InitialsAvatar
              initials={initialsFor(title)}
              slot={avatarSlot(currentConversation)}
              size={32}
              testID="chat-header-avatar"
            />
          </View>
        )}
        <View className="flex-1">
          <Text className="text-lg font-semibold text-text-primary" numberOfLines={1} testID="chat-title">
            {title}
          </Text>
          {handle && (
            <Text className="text-xs text-text-tertiary" numberOfLines={1} testID="chat-header-handle">
              {MENTION_PREFIX}
              {handle}
            </Text>
          )}
          {!handle && kind === 'group' && currentConversation?.group_name && (
            <Text className="text-xs text-text-tertiary" numberOfLines={1} testID="chat-header-group">
              {currentConversation.group_name}
            </Text>
          )}
        </View>
        {currentConversation && <Text className="text-[10px] ml-1 text-text-tertiary">▼</Text>}
      </TouchableOpacity>

      {/* Quick appearance toggle (sun/moon) — flips persisted Light/Dark pref */}
      <AppearanceToggleButton size={20} color={colors.text.secondary} />

      {/* Notification bell */}
      <NotificationBellButton size={20} color={colors.text.secondary} />

      {/* The chat "+" */}
      <TouchableOpacity
        className="w-10 h-10 items-center justify-center"
        onPress={onPlusPress}
        accessibilityRole="button"
        accessibilityLabel={t('app.headerPlusAria')}
        testID="chat-plus-button"
      >
        <Ionicons name="add" size={26} color={colors.pierre.violet} />
      </TouchableOpacity>
    </View>
  );
}
