// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Renders the group's shared room transcript on mobile, member-gated and consent-filtered
// ABOUTME: The same read model the coach's ambient context uses - one room across every surface

import React from 'react';
import { ActivityIndicator, Text, View } from 'react-native';
import { useThemeColors } from '../../constants/theme';
import { useGroupTranscript } from '../../hooks/useGroups';
import { useTranslation } from '@pierre/i18n';

interface GroupTranscriptSectionProps {
  /** The group whose room to read. */
  groupId: string;
}

/**
 * The room, as every member shares it.
 *
 * Entries arrive consent-filtered from the server: an unconsented member stays
 * on the roster while their words are withheld — the same rule the pipeline
 * applies before the coach reasons over the room.
 *
 * No card: this mounts inside `GroupInfoSheet`'s Room `CollapsibleSection`,
 * whose own header already reads "Room" — an inner heading repeating that
 * word made sense as a card's own label but is redundant once flattened, so
 * it is dropped here rather than carried into a plain view.
 */
export function GroupTranscriptSection({ groupId }: GroupTranscriptSectionProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const { transcript, isLoading, isError } = useGroupTranscript(groupId, true);

  if (isLoading) {
    return (
      <View className="items-center py-4" testID="group-transcript-loading">
        <ActivityIndicator />
      </View>
    );
  }

  if (isError || !transcript) {
    return (
      <View className="py-4">
        <Text className="text-sm" style={{ color: colors.text.tertiary, textAlign: 'center' }}>
          {t('app.roomTranscriptFailed')}
        </Text>
      </View>
    );
  }

  return (
    <View testID="group-transcript">
      {transcript.entries.length === 0 ? (
        <Text className="text-sm" style={{ color: colors.text.tertiary }}>
          {t('app.roomEmpty')}
        </Text>
      ) : (
        transcript.entries.map((entry) => (
          <View key={entry.id} style={{ marginBottom: 10 }}>
            <View style={{ flexDirection: 'row', justifyContent: 'space-between' }}>
              <Text className="text-xs font-semibold" style={{ color: colors.text.secondary }}>
                {entry.author_display_name ?? entry.author_user_id}
                {entry.speaker === 'coach' ? ' · agent' : ''}
              </Text>
              <Text className="text-xs font-mono tabular-nums" style={{ color: colors.text.tertiary }}>
                {new Date(entry.created_at).toLocaleDateString()}
              </Text>
            </View>
            <Text className="text-sm" style={{ color: colors.text.primary, marginTop: 2 }}>
              {entry.content}
            </Text>
          </View>
        ))
      )}
    </View>
  );
}
