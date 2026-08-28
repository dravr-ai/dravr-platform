// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Bottom sheet behind the chat screens' "+" — new chat, new group chat, add someone to this discussion
// ABOUTME: Lists exactly the actions useChatPlusActions decides; the flows they open are the host's

import React from 'react';
import { View, Text, TouchableOpacity, Modal } from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import { useThemeColors } from '../../constants/theme';
import type { ChatPlusAction } from './useChatPlusActions';
import { useTranslation } from '@pierre/i18n';

interface ChatPlusSheetProps {
  visible: boolean;
  onClose: () => void;
  actions: ChatPlusAction[];
}

/**
 * The Telegram-shaped compose menu.
 *
 * A row closes the sheet before it acts, so an action that opens another
 * modal (the group picker, the participants sheet) never stacks on top of
 * this one.
 */
export function ChatPlusSheet({ visible, onClose, actions }: ChatPlusSheetProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();

  return (
    <Modal visible={visible} animationType="slide" transparent onRequestClose={onClose}>
      <TouchableOpacity
        className="flex-1 justify-end bg-black/40"
        activeOpacity={1}
        onPress={onClose}
        testID="chat-plus-sheet-backdrop"
      >
        <View
          className="rounded-t-2xl px-4 pt-4 pb-8"
          style={{ backgroundColor: colors.background.secondary }}
          onStartShouldSetResponder={() => true}
          testID="chat-plus-sheet"
        >
          <View className="flex-row items-center justify-between mb-2">
            <Text className="text-lg font-semibold text-text-primary">{t('app.start')}</Text>
            <TouchableOpacity onPress={onClose} testID="chat-plus-close" accessibilityLabel={t('common.close')}>
              <Ionicons name="close" size={22} color={colors.text.secondary} />
            </TouchableOpacity>
          </View>

          {actions.map((action) => {
            const Icon = action.icon;
            return (
              <TouchableOpacity
                key={action.id}
                className="flex-row items-center py-3"
                onPress={() => {
                  onClose();
                  action.onPress();
                }}
                accessibilityRole="button"
                accessibilityLabel={action.label}
                testID={`chat-plus-action-${action.id}`}
              >
                <Icon size={22} color={colors.pierre.violet} />
                <Text className="ml-3 text-base text-text-primary">{action.label}</Text>
              </TouchableOpacity>
            );
          })}
        </View>
      </TouchableOpacity>
    </Modal>
  );
}
