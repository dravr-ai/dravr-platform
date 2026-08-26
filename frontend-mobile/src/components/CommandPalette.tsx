// ABOUTME: Slash-command autocomplete rendered above the mobile composer
// ABOUTME: Draws the server's per-caller catalogue; holds no command list of its own

import React from 'react';
import { View, Text, ScrollView, TouchableOpacity } from 'react-native';
import type { CommandEntry } from '@pierre/shared-types';
import { useThemeColors } from '../constants/theme';

export interface CommandPaletteProps {
  /** The commands to offer, already filtered and ordered by the server. */
  matches: CommandEntry[];
  /** Fill the composer with this command. */
  onSelect: (entry: CommandEntry) => void;
}

/**
 * The `/` autocomplete over the composer.
 *
 * Every row is a command the server said this caller may run — the listing is
 * resolved per caller by the same availability predicates `/help` asks, so an
 * athlete in no group is never shown `/group invite`. Renders nothing when
 * there is nothing to offer, which is what closes it.
 */
export function CommandPalette({ matches, onSelect }: CommandPaletteProps) {
  const colors = useThemeColors();

  if (matches.length === 0) return null;

  return (
    <View
      testID="command-palette"
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
        {matches.map((entry, index) => (
          <TouchableOpacity
            key={entry.name}
            testID={`command-palette-option-${entry.name}`}
            onPress={() => onSelect(entry)}
            style={{
              paddingHorizontal: 16,
              paddingVertical: 10,
              borderBottomWidth: index < matches.length - 1 ? 1 : 0,
              borderBottomColor: colors.border.subtle,
            }}
          >
            <View style={{ flexDirection: 'row', alignItems: 'baseline' }}>
              <Text style={{ fontSize: 14, color: colors.text.primary, fontWeight: '600' }}>
                {entry.command}
              </Text>
              {entry.args !== null && (
                <Text style={{ fontSize: 13, color: colors.text.tertiary }}> {entry.args}</Text>
              )}
              <Text
                style={{
                  marginLeft: 'auto',
                  fontSize: 10,
                  textTransform: 'uppercase',
                  color: colors.text.tertiary,
                }}
              >
                {entry.domain}
              </Text>
            </View>
            <Text style={{ fontSize: 12, color: colors.text.secondary, marginTop: 2 }} numberOfLines={1}>
              {entry.description}
            </Text>
          </TouchableOpacity>
        ))}
      </ScrollView>
    </View>
  );
}
