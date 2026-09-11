// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Renders the group's shared room transcript on mobile, member-gated and consent-filtered
// ABOUTME: The same read model the coach's ambient context uses - one room across every surface

import React from 'react';
import { ActivityIndicator, Text, View, type ViewStyle } from 'react-native';
import { useCardStyle, useThemeColors } from '../../constants/theme';
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
 */
export function GroupTranscriptSection({ groupId }: GroupTranscriptSectionProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const sectionCardStyle: ViewStyle = {
    borderRadius: 12,
    padding: 14,
    ...useCardStyle(),
  };
  const { transcript, isLoading, isError } = useGroupTranscript(groupId, true);

  if (isLoading) {
    return (
      <View style={sectionCardStyle} testID="group-transcript-loading">
        <ActivityIndicator />
      </View>
    );
  }

  if (isError || !transcript) {
    return (
      <View style={sectionCardStyle}>
        <Text className="text-sm" style={{ color: colors.text.tertiary, textAlign: 'center' }}>
          {t('app.roomTranscriptFailed')}
        </Text>
      </View>
    );
  }

  return (
    <View style={sectionCardStyle} testID="group-transcript">
      <Text className="text-base font-semibold" style={{ color: colors.text.primary, marginBottom: 8 }}>
        {t('app.room')}
      </Text>
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
