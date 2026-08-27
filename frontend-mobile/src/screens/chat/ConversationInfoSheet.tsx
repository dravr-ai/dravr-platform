// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The bottom sheet behind the chat header's title — group info, coach info, or the plain thread rows
// ABOUTME: One host for the three shapes a thread can have, so tapping the title always opens the same place

import React from 'react';
import { View, Text, TouchableOpacity, Modal } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { deriveKind } from '@pierre/chat-utils';
import { useThemeColors } from '../../constants/theme';
import { DragIndicator } from '../../components/ui';
import type { Conversation } from '../../types';
import { GroupInfoSheet } from '../groups/GroupInfoSheet';
import { CoachInfoSheet } from './CoachInfoSheet';

export interface ConversationInfoSheetProps {
  visible: boolean;
  /** The open thread; the sheet renders nothing without one. */
  conversation: Conversation | null;
  onClose: () => void;
  /** Send a command as the next turn — how Coach info detaches its coach. */
  onSendCommand: (command: string) => void;
  onRename: () => void;
  onParticipants: () => void;
  onDelete: () => void;
  /** The thread is no longer the athlete's: go back to the conversation list. */
  onLeaveThread: () => void;
}

/**
 * What tapping a thread's title opens.
 *
 * Three shapes, decided by what the conversation *is* rather than by which
 * screen opened it: a group thread gets Group info, a coach-bound thread gets
 * Coach info, and a plain thread gets the rename / participants / delete rows
 * that used to live in the header popover.
 */
export function ConversationInfoSheet({
  visible,
  conversation,
  onClose,
  onSendCommand,
  onRename,
  onParticipants,
  onDelete,
  onLeaveThread,
}: ConversationInfoSheetProps) {
  const colors = useThemeColors();
  if (!conversation) return null;
  const kind = deriveKind(conversation);

  return (
    <Modal visible={visible} animationType="slide" transparent onRequestClose={onClose}>
      <TouchableOpacity
        className="flex-1 justify-end bg-black/40"
        activeOpacity={1}
        onPress={onClose}
        testID="conversation-info-backdrop"
      >
        <View
          className="rounded-t-2xl px-4 pt-3 pb-8 max-h-[85%]"
          style={{ backgroundColor: colors.background.secondary }}
          onStartShouldSetResponder={() => true}
          testID="conversation-info-sheet"
        >
          <DragIndicator />
          <View className="flex-row items-center justify-end mb-1">
            <TouchableOpacity onPress={onClose} accessibilityLabel="Close" testID="conversation-info-close">
              <Feather name="x" size={22} color={colors.text.secondary} />
            </TouchableOpacity>
          </View>

          {kind === 'group' && conversation.group_id ? (
            <GroupInfoSheet
              groupId={conversation.group_id}
              fallbackName={conversation.group_name ?? conversation.title ?? null}
              onClose={onClose}
              onLeft={onLeaveThread}
            />
          ) : conversation.coach_id ? (
            <CoachInfoSheet
              coachId={conversation.coach_id}
              fallbackTitle={conversation.coach_title ?? null}
              onSendCommand={onSendCommand}
              onClose={onClose}
            />
          ) : (
            <View testID="conversation-info-plain">
              <Text className="text-lg font-bold text-text-primary" testID="conversation-info-title">
                {conversation.title || 'Untitled chat'}
              </Text>
              <TouchableOpacity
                className="flex-row items-center py-3 mt-2"
                onPress={onRename}
                accessibilityRole="button"
                testID="conversation-info-rename"
              >
                <Feather name="edit-2" size={18} color={colors.text.primary} />
                <Text className="text-base text-text-primary ml-3">Rename</Text>
              </TouchableOpacity>
              <TouchableOpacity
                className="flex-row items-center py-3"
                onPress={onParticipants}
                accessibilityRole="button"
                testID="conversation-info-participants"
              >
                <Feather name="users" size={18} color={colors.text.primary} />
                <Text className="text-base text-text-primary ml-3">Participants</Text>
              </TouchableOpacity>
              <TouchableOpacity
                className="flex-row items-center py-3"
                onPress={onDelete}
                accessibilityRole="button"
                testID="conversation-info-delete"
              >
                <Feather name="trash-2" size={18} color={colors.error} />
                <Text className="text-base text-error ml-3">Delete</Text>
              </TouchableOpacity>
            </View>
          )}
        </View>
      </TouchableOpacity>
    </Modal>
  );
}
