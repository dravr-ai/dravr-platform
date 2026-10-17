// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The bottom sheet behind the chat header's title — group info, a plan thread's athlete, agent info, or the plain thread rows
// ABOUTME: One host for the four shapes a thread can have, so tapping the title always opens the same place

import React from 'react';
import { View, Text, TouchableOpacity } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { avatarSlot, deriveKind, initialsFor, PLAN_THREAD_KEYS } from '@pierre/chat-utils';
import { useThemeColors } from '../../constants/theme';
import { Sheet } from '../../components/ui';
import { InitialsAvatar } from '../../components/ui/InitialsAvatar';
import { useGroup } from '../../hooks/useGroups';
import type { Conversation } from '../../types';
import { GroupInfoSheet } from '../groups/GroupInfoSheet';
import { CoachInfoSheet } from './CoachInfoSheet';
import { useTranslation } from '@pierre/i18n';

/**
 * A coach's plan thread, described: the athlete whose plan it works on, and
 * the group whose coaching link scopes it. Nothing here manages the group —
 * that is Group info's, in the group's own thread.
 */
function SubjectInfo({ conversation }: { conversation: Conversation }) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const { group } = useGroup(conversation.subject_group_id ?? '');
  const name = conversation.subject_display_name?.trim() || t('app.unknownMember');
  return (
    <View testID="subject-info-sheet">
      <View className="flex-row items-center">
        <InitialsAvatar initials={initialsFor(name)} slot={avatarSlot(conversation)} size={40} />
        <View className="ml-3 shrink">
          <Text className="text-lg font-bold text-text-primary" numberOfLines={1} testID="subject-info-name">
            {name}
          </Text>
          <Text className="text-sm text-text-secondary" numberOfLines={1}>
            {t(PLAN_THREAD_KEYS.kindLabel)}
          </Text>
        </View>
      </View>
      {group ? (
        <View className="flex-row items-center py-3 mt-2">
          <Feather name="users" size={18} color={colors.text.secondary} />
          <View className="ml-3 shrink">
            <Text className="text-xs text-text-tertiary">{t('app.group')}</Text>
            <Text className="text-base text-text-primary" numberOfLines={1} testID="subject-info-group">
              {group.name}
            </Text>
          </View>
        </View>
      ) : null}
    </View>
  );
}

export interface ConversationInfoSheetProps {
  visible: boolean;
  /** The open thread; the sheet renders nothing without one. */
  conversation: Conversation | null;
  onClose: () => void;
  /** Send a command as the next turn — how Agent info detaches its agent. */
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
 * Four shapes, decided by what the conversation *is* rather than by which
 * screen opened it: a group thread gets Group info, a coach's plan thread
 * gets its athlete and their group (before the agent it runs on), an
 * agent-bound thread gets Agent info, and a plain thread gets the rename /
 * participants / delete rows that used to live in the header popover.
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
  const { t } = useTranslation();
  const colors = useThemeColors();
  if (!conversation) return null;
  const kind = deriveKind(conversation);

  return (
    <Sheet
      visible={visible}
      onClose={onClose}
      testID="conversation-info-sheet"
      backdropTestID="conversation-info-backdrop"
    >
      <View className="flex-row items-center justify-end mb-1">
        <TouchableOpacity onPress={onClose} accessibilityLabel={t('common.close')} testID="conversation-info-close">
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
      ) : kind === 'subject' ? (
        <SubjectInfo conversation={conversation} />
      ) : conversation.agent_id ? (
        <CoachInfoSheet
          agentId={conversation.agent_id}
          fallbackTitle={conversation.agent_title ?? null}
          onSendCommand={onSendCommand}
          onClose={onClose}
        />
      ) : (
        <View testID="conversation-info-plain">
          <Text className="text-lg font-bold text-text-primary" testID="conversation-info-title">
            {conversation.title || t('app.untitledChat')}
          </Text>
          <TouchableOpacity
            className="flex-row items-center py-3 mt-2"
            onPress={onRename}
            accessibilityRole="button"
            testID="conversation-info-rename"
          >
            <Feather name="edit-2" size={18} color={colors.text.primary} />
            <Text className="text-base text-text-primary ml-3">{t('app.convMenuRename')}</Text>
          </TouchableOpacity>
          <TouchableOpacity
            className="flex-row items-center py-3"
            onPress={onParticipants}
            accessibilityRole="button"
            testID="conversation-info-participants"
          >
            <Feather name="users" size={18} color={colors.text.primary} />
            <Text className="text-base text-text-primary ml-3">{t('app.participants')}</Text>
          </TouchableOpacity>
          <TouchableOpacity
            className="flex-row items-center py-3"
            onPress={onDelete}
            accessibilityRole="button"
            testID="conversation-info-delete"
          >
            <Feather name="trash-2" size={18} color={colors.error} />
            <Text className="text-base text-error ml-3">{t('common.delete')}</Text>
          </TouchableOpacity>
        </View>
      )}
    </Sheet>
  );
}
