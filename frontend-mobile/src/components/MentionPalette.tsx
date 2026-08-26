// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: @handle autocomplete rendered above the mobile composer — the athlete's installed coaches
// ABOUTME: Offers only coaches the server lists as installed; holds no roster of its own

import React from 'react';
import { View, Text, ScrollView, TouchableOpacity } from 'react-native';
import type { MentionCandidate } from '@pierre/shared-constants';
import { useThemeColors } from '../constants/theme';

export interface MentionPaletteProps {
  /** The installed coaches whose handle matches what is being typed. */
  matches: MentionCandidate[];
  /** Fill the composer with this coach's handle, lowercase and verbatim. */
  onSelect: (candidate: MentionCandidate) => void;
}

/**
 * The `@` autocomplete over the composer.
 *
 * Every row is a coach on the athlete's own list — the only coaches a mention
 * routes to for one turn. Renders nothing when there is nothing to offer,
 * which is what closes it, the same way the slash palette closes.
 */
export function MentionPalette({ matches, onSelect }: MentionPaletteProps) {
  const colors = useThemeColors();

  if (matches.length === 0) return null;

  return (
    <View
      testID="mention-palette"
      style={{
        maxHeight: 220,
        marginBottom: 8,
        borderRadius: 16,
        borderWidth: 1,
        borderColor: colors.border.default,
        backgroundColor: colors.background.tertiary,
        overflow: 'hidden',
      }}
    >
      <ScrollView keyboardShouldPersistTaps="always">
        {matches.map((coach, index) => (
          <TouchableOpacity
            key={coach.handle}
            testID={`mention-palette-option-${coach.handle}`}
            accessibilityRole="button"
            accessibilityLabel={`Mention @${coach.handle}`}
            onPress={() => onSelect(coach)}
            style={{
              paddingHorizontal: 16,
              paddingVertical: 10,
              borderBottomWidth: index < matches.length - 1 ? 1 : 0,
              borderBottomColor: colors.border.subtle,
            }}
          >
            <View style={{ flexDirection: 'row', alignItems: 'baseline' }}>
              <Text style={{ fontSize: 14, color: colors.text.primary, fontWeight: '600' }}>
                @{coach.handle}
              </Text>
              <Text
                style={{ marginLeft: 8, fontSize: 13, color: colors.text.secondary, flexShrink: 1 }}
                numberOfLines={1}
              >
                {coach.title}
              </Text>
            </View>
          </TouchableOpacity>
        ))}
      </ScrollView>
    </View>
  );
}
