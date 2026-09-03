// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Chat header — back, the thread's initials avatar, its title, the appearance toggle and the bell
// ABOUTME: Tapping the avatar or the title opens the thread's info sheet, the way every messaging app does it

import React from 'react';
import { useTranslation } from '@pierre/i18n';
import { View, Text, TouchableOpacity } from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import {
  avatarSlot,
  CONVERSATION_ROW_LABEL_KEYS,
  initialsFor,
  threadSubtitle,
} from '@pierre/chat-utils';
import { MENTION_PREFIX } from '@pierre/shared-constants';
import { spacing, useThemeColors } from '../../constants/theme';
import type { Conversation } from '../../types';
import { NotificationBellButton } from '../../components/notifications/NotificationBellButton';
import { AppearanceToggleButton } from '../../components/ui/AppearanceToggleButton';
import { InitialsAvatar } from '../../components/ui/InitialsAvatar';

interface ChatHeaderProps {
  currentConversation: Conversation | null;
  insetTop: number;
  /** Back to the conversation list the thread was opened from. */
  onBackPress: () => void;
  /** Open the thread's info sheet — group info, coach info, or the plain rows. */
  onTitlePress: () => void;
}

export function ChatHeader({
  currentConversation,
  insetTop,
  onBackPress,
  onTitlePress,
}: ChatHeaderProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  // An open thread with no title reads as untitled; before a thread exists the
  // header names what the athlete is about to start.
  const title =
    currentConversation?.title?.trim() ||
    (currentConversation ? t(CONVERSATION_ROW_LABEL_KEYS.untitled) : t('chat.newChat'));
  // Group before handle — the precedence both headers now share.
  const subtitle = threadSubtitle(currentConversation);

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
          {subtitle?.kind === 'group' && currentConversation?.group_name && (
            <Text className="text-xs text-text-tertiary" numberOfLines={1} testID="chat-header-group">
              {currentConversation.group_name}
            </Text>
          )}
          {subtitle?.kind === 'handle' && (
            <Text className="text-xs text-text-tertiary" numberOfLines={1} testID="chat-header-handle">
              {MENTION_PREFIX}
              {subtitle.handle}
            </Text>
          )}
        </View>
        {currentConversation && <Text className="text-[10px] ml-1 text-text-tertiary">▼</Text>}
      </TouchableOpacity>

      {/* Quick appearance toggle (sun/moon) — flips persisted Light/Dark pref */}
      <AppearanceToggleButton size={20} color={colors.text.secondary} />

      {/*
        Notification bell. Nothing follows it: starting a discussion belongs
        to the tab bar's "+", which is the app's one entry point for it and
        the one within reach of a thumb (carnet#213).
      */}
      <NotificationBellButton size={20} color={colors.text.secondary} />
    </View>
  );
}
