// ABOUTME: Chat header component with navigation, title, and action menu
// ABOUTME: Displays coach avatar, conversation title, and new chat button

import React from 'react';
import { View, Text, TouchableOpacity, Modal, Image } from 'react-native';
import type { ViewStyle } from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import { spacing, useThemeColors } from '../../constants/theme';
import type { Conversation } from '../../types';
import { NotificationBellButton } from '../../components/notifications/NotificationBellButton';
import { AppearanceToggleButton } from '../../components/ui/AppearanceToggleButton';

// popoverContainerStyle is built inside the component so its background and
// shadow track the active scheme (it's a floating menu — needs surface
// contrast in both modes). Layout stays constant.
const POPOVER_LAYOUT: ViewStyle = {
  position: 'absolute',
  top: 68,
  left: 60,
  right: 60,
  borderRadius: 12,
  paddingVertical: spacing.xs,
  shadowOffset: { width: 0, height: 8 },
  shadowRadius: 16,
  elevation: 12,
};

interface ChatHeaderProps {
  currentConversation: Conversation | null;
  actionMenuVisible: boolean;
  insetTop: number;
  onHistoryPress: () => void;
  onTitlePress: () => void;
  onMenuClose: () => void;
  onMenuRename: () => void;
  onMenuDelete: () => void;
}

export function ChatHeader({
  currentConversation,
  actionMenuVisible,
  insetTop,
  onHistoryPress,
  onTitlePress,
  onMenuClose,
  onMenuRename,
  onMenuDelete,
}: ChatHeaderProps) {
  const colors = useThemeColors();
  const popoverContainerStyle: ViewStyle = {
    ...POPOVER_LAYOUT,
    backgroundColor: colors.background.secondary,
    shadowColor: colors.text.primary,
    shadowOpacity: 0.4,
  };
  return (
    <>
      <View
        className="flex-row items-center px-4 py-2 border-b border-border-subtle"
        style={{ paddingTop: insetTop + spacing.sm }}
      >
        {/* History button */}
        <TouchableOpacity
          className="w-10 h-10 items-center justify-center"
          onPress={onHistoryPress}
          testID="history-button"
        >
          <Ionicons
            name="time-outline"
            size={24}
            color={colors.text.primary}
          />
        </TouchableOpacity>

        {/* Coach avatar with status dot when in conversation */}
        {currentConversation && (
          <View className="relative mr-2">
            <View className="w-10 h-10 rounded-full overflow-hidden bg-surface-container-low items-center justify-center">
              <Image
                source={require('../../../assets/dravr-logo.png')}
                className="w-10 h-10"
                resizeMode="cover"
              />
            </View>
            {/* Pulsing green status dot per Stitch spec */}
            <View className="absolute bottom-0 right-0 w-3 h-3 rounded-full bg-pierre-activity border-2 border-background-primary" />
          </View>
        )}

        <TouchableOpacity
          className={`flex-1 flex-row items-center ${currentConversation ? '' : 'justify-center'} mx-2 ${actionMenuVisible ? 'opacity-0' : ''}`}
          onPress={onTitlePress}
          disabled={!currentConversation}
          testID="chat-title-button"
        >
          <Text className="text-lg font-semibold text-text-primary" numberOfLines={1} testID="chat-title">
            {currentConversation?.title || 'New Chat'}
          </Text>
          {currentConversation && (
            <Text className="text-[10px] ml-1 text-text-tertiary">▼</Text>
          )}
        </TouchableOpacity>

        {/* Quick appearance toggle (sun/moon) — flips persisted Light/Dark pref */}
        <AppearanceToggleButton size={20} color={colors.text.secondary} />

        {/* Notification bell */}
        <NotificationBellButton size={20} color={colors.text.secondary} />
      </View>

      {/* Conversation Action Menu Modal - Claude-style popover */}
      <Modal
        visible={actionMenuVisible}
        animationType="fade"
        transparent
        onRequestClose={onMenuClose}
      >
        <TouchableOpacity
          className="flex-1 bg-black/30"
          activeOpacity={1}
          onPress={onMenuClose}
        >
          <View style={popoverContainerStyle}>
            <TouchableOpacity
              className="flex-row items-center px-4 py-3 opacity-40"
              disabled
            >
              <Ionicons name="star-outline" size={20} color={colors.text.tertiary} style={{ marginRight: spacing.md, width: 24 }} />
              <Text className="text-base text-text-tertiary">Add to favorites</Text>
            </TouchableOpacity>

            <TouchableOpacity
              className="flex-row items-center px-4 py-3"
              onPress={onMenuRename}
            >
              <Ionicons name="pencil-outline" size={20} color={colors.text.primary} style={{ marginRight: spacing.md, width: 24 }} />
              <Text className="text-base text-text-primary">Rename</Text>
            </TouchableOpacity>

            <TouchableOpacity
              className="flex-row items-center px-4 py-3"
              onPress={onMenuDelete}
            >
              <Ionicons name="trash-outline" size={20} color={colors.error} style={{ marginRight: spacing.md, width: 24 }} />
              <Text className="text-base text-error">Delete</Text>
            </TouchableOpacity>
          </View>
        </TouchableOpacity>
      </Modal>
    </>
  );
}
