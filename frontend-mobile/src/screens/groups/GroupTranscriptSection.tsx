// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Renders the group's shared room transcript on mobile, member-gated and consent-filtered
// ABOUTME: The same read model the coach's ambient context uses - one room across every surface

import React from 'react';
import { ActivityIndicator, Text, View, type ViewStyle } from 'react-native';
import { glassCard, useThemeColors } from '../../constants/theme';

const sectionCardStyle: ViewStyle = {
  borderRadius: 12,
  padding: 14,
  ...glassCard,
};
import { useGroupTranscript } from '../../hooks/useGroups';

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
  const colors = useThemeColors();
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
        <Text style={{ fontSize: 13, color: colors.text.tertiary, textAlign: 'center' }}>
          The room transcript could not be loaded.
        </Text>
      </View>
    );
  }

  return (
    <View style={sectionCardStyle} testID="group-transcript">
      <Text style={{ fontSize: 15, fontWeight: '600', color: colors.text.primary, marginBottom: 8 }}>
        Room
      </Text>
      {transcript.entries.length === 0 ? (
        <Text style={{ fontSize: 13, color: colors.text.tertiary }}>
          Nothing said in the room yet. Messages from every surface land here once the group starts
          talking.
        </Text>
      ) : (
        transcript.entries.map((entry) => (
          <View key={entry.id} style={{ marginBottom: 10 }}>
            <View style={{ flexDirection: 'row', justifyContent: 'space-between' }}>
              <Text style={{ fontSize: 12, fontWeight: '600', color: colors.text.secondary }}>
                {entry.author_display_name ?? entry.author_user_id}
                {entry.speaker === 'coach' ? ' · coach' : ''}
              </Text>
              <Text style={{ fontSize: 11, color: colors.text.tertiary }}>
                {new Date(entry.created_at).toLocaleDateString()}
              </Text>
            </View>
            <Text style={{ fontSize: 14, color: colors.text.primary, marginTop: 2 }}>
              {entry.content}
            </Text>
          </View>
        ))
      )}
    </View>
  );
}
