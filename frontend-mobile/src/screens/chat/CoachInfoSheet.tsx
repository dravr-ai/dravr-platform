// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Coach info for a coach-bound thread — title, @handle, description, detach, and edit for own coaches
// ABOUTME: "Remove from this chat" sends /coach remove; the command is the only implementation of detaching

import React, { useCallback } from 'react';
import { View, Text, TouchableOpacity, ActivityIndicator, ScrollView } from 'react-native';
import { useRouter } from 'expo-router';
import { Feather } from '@expo/vector-icons';
import { COMMAND_DRAFTS, MENTION_PREFIX } from '@pierre/shared-constants';
import { useThemeColors } from '../../constants/theme';
import { COACH_EDIT_ROUTE } from '../../navigation/routes';
import { useCoachInfo } from '../../hooks/useCoachInfo';

export interface CoachInfoSheetProps {
  /** The coach the open thread is bound to. */
  coachId: string;
  /** The coach's title as the conversation row spells it, until the list loads. */
  fallbackTitle: string | null;
  /** Send a command as the next turn of this thread. */
  onSendCommand: (command: string) => void;
  /** Close the host sheet — an action that navigates away closes it first. */
  onClose: () => void;
}

/**
 * What a coach-bound thread says about its coach.
 *
 * The handle is the point: it is how the athlete brings this coach into any
 * other conversation. Detaching is `/coach remove`, sent as a turn, so the
 * app has no private path to a state the command cannot reach.
 */
export function CoachInfoSheet({ coachId, fallbackTitle, onSendCommand, onClose }: CoachInfoSheetProps) {
  const colors = useThemeColors();
  const router = useRouter();
  const { coach, isLoading } = useCoachInfo(coachId);

  const handleRemove = useCallback(() => {
    onClose();
    onSendCommand(COMMAND_DRAFTS.coachRemove);
  }, [onClose, onSendCommand]);

  const handleEdit = useCallback(() => {
    onClose();
    router.push({ pathname: COACH_EDIT_ROUTE, params: { coachId } });
  }, [onClose, router, coachId]);

  return (
    <ScrollView testID="coach-info-sheet" keyboardShouldPersistTaps="handled">
      <Text className="text-lg font-bold text-text-primary" testID="coach-info-title">
        {coach?.title ?? fallbackTitle ?? 'Coach'}
      </Text>

      {coach?.handle && (
        <Text className="text-sm text-text-secondary mt-1" testID="coach-info-handle">
          {MENTION_PREFIX}
          {coach.handle}
        </Text>
      )}

      {isLoading && <ActivityIndicator className="mt-3" size="small" color={colors.pierre.violet} />}

      {coach?.description && (
        <Text className="text-sm text-text-secondary leading-5 mt-3" testID="coach-info-description">
          {coach.description}
        </Text>
      )}

      {coach?.handle && (
        <Text className="text-xs text-text-tertiary mt-3">
          Mention {MENTION_PREFIX}
          {coach.handle} in any chat to bring this coach in for one turn.
        </Text>
      )}

      <View className="mt-5">
        <TouchableOpacity
          className="flex-row items-center py-3"
          onPress={handleRemove}
          accessibilityRole="button"
          testID="coach-info-remove"
        >
          <Feather name="user-minus" size={18} color={colors.text.primary} />
          <Text className="text-base text-text-primary ml-3">Remove from this chat</Text>
        </TouchableOpacity>

        {/* Only the athlete's own coaches are editable; a system coach is
            shared by every tenant and the server refuses the write. */}
        {coach && !coach.is_system && (
          <TouchableOpacity
            className="flex-row items-center py-3"
            onPress={handleEdit}
            accessibilityRole="button"
            testID="coach-info-edit"
          >
            <Feather name="edit-2" size={18} color={colors.text.primary} />
            <Text className="text-base text-text-primary ml-3">Edit coach</Text>
          </TouchableOpacity>
        )}
      </View>
    </ScrollView>
  );
}
