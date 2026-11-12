// ABOUTME: The envelope notice under a reply: what the envelope check found on the turn's sessions
// ABOUTME: The platform's own translated sentences, drawn as written; the agent's prose explains, this states

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React from 'react';
import { Text, View } from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import { useThemeColors } from '../../constants/theme';

interface EnvelopeNoticeProps {
  /** The notice's heading, as the server rendered it in the athlete's locale. */
  heading: string;
  /** One line per session: its day, what was found, a relabel and its anchor. */
  lines: string[];
}

/**
 * A session saved as written but outside what the evidence supports
 * (carnet#1495). Every string arrives rendered: the client lays it out and
 * adds no words of its own.
 */
export default function EnvelopeNotice({ heading, lines }: EnvelopeNoticeProps) {
  const colors = useThemeColors();
  return (
    <View
      accessibilityRole="summary"
      accessibilityLabel={heading}
      className="mt-3 rounded-lg border border-warning/30 bg-warning/10 px-3 py-2"
    >
      <View className="flex-row items-center gap-2">
        <Ionicons name="warning-outline" size={16} color={colors.warning} />
        <Text className="flex-1 text-sm font-medium text-on-warning-container">{heading}</Text>
      </View>
      {lines.map((line, index) => (
        <Text key={`${index}-${line.slice(0, 24)}`} className="mt-1 pl-6 text-sm text-text-primary">
          {line}
        </Text>
      ))}
    </View>
  );
}
