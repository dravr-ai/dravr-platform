// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The line over a stretch of a coach's private rows in a one-athlete room — "Only you and <agent> see what follows"
// ABOUTME: Dashed in the private accent either side of a lock and the label, as the private bubbles under it are (carnet#1249)

import React from 'react';
import { Text, View } from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import { useThemeColors } from '../../constants/theme';

/**
 * Where the room on screen stops being the room: what follows, up to the next
 * row the room reads, is the coach's work thread with the agent, which the
 * athlete never fetches. Drawn above every such stretch, so a scroll that
 * lands in the middle of one still finds where it began.
 */
export default function PrivateLaneDivider({ label }: { label: string }) {
  const colors = useThemeColors();
  // Each rule is a one-point box dashed on every side: a dashed border drawn
  // on one side only falls back to solid on both platforms.
  return (
    <View
      className="mt-4 mb-1 flex-row items-center gap-2"
      accessible
      accessibilityLabel={label}
      testID="private-lane-divider"
    >
      <View className="flex-1 h-px rounded-sm border border-dashed border-warning/60" />
      <Ionicons name="lock-closed" size={12} color={colors.ink.warning} />
      <Text className="text-xs font-medium text-on-warning-container">{label}</Text>
      <View className="flex-1 h-px rounded-sm border border-dashed border-warning/60" />
    </View>
  );
}
