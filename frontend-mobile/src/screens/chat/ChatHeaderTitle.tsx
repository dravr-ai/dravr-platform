// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The thread's title view inside the native header — its initials avatar, its title and one subtitle line
// ABOUTME: Tapping it opens the thread's info sheet, the way every messaging app does it; the bar itself is the system's

import React from 'react';
import { useTranslation } from '@pierre/i18n';
import { View, Text, TouchableOpacity } from 'react-native';
import {
  avatarSlot,
  CONVERSATION_ROW_LABEL_KEYS,
  initialsFor,
  threadSubtitle,
} from '@pierre/chat-utils';
import { MENTION_PREFIX } from '@pierre/shared-constants';
import type { Conversation } from '../../types';
import { InitialsAvatar } from '../../components/ui/InitialsAvatar';

interface ChatHeaderTitleProps {
  currentConversation: Conversation | null;
  /**
   * What the coach can see, for a thread with no group and no handle to name.
   * `null` while the status call is still in flight.
   */
  providerStatus: string | null;
  /** Open the thread's info sheet — group info, coach info, or the plain rows. */
  onTitlePress: () => void;
}

export function ChatHeaderTitle({ currentConversation, providerStatus, onTitlePress }: ChatHeaderTitleProps) {
  const { t } = useTranslation();
  // An open thread with no title reads as untitled; before a thread exists the
  // header names what the athlete is about to start.
  const title =
    currentConversation?.title?.trim() ||
    (currentConversation ? t(CONVERSATION_ROW_LABEL_KEYS.untitled) : t('chat.newChat'));
  // Group before handle — the precedence both headers now share.
  const subtitle = threadSubtitle(currentConversation);

  return (
    <TouchableOpacity
      className="flex-row items-center"
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
      <View className="shrink">
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
        {/*
          Neither a group nor a coach handle to name, so the line says what
          the coach can actually read. Web has said this for a while; the
          phone said nothing, which left an athlete whose provider session
          had died with no explanation for a coach gone quiet (carnet#231).
        */}
        {!subtitle && providerStatus && (
          <Text
            className="text-xs text-text-tertiary"
            numberOfLines={1}
            testID="chat-header-provider-status"
          >
            {providerStatus}
          </Text>
        )}
      </View>
    </TouchableOpacity>
  );
}
