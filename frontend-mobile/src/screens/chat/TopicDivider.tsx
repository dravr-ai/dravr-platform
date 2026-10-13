// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The rule where a new topic opens in the personal thread — "+" or /reset drew it, the time it opened beside it
// ABOUTME: The React Native half of the web divider: a hairline either side of a quiet label

import React from 'react';
import { View, Text } from 'react-native';

/**
 * Where a new topic opens in the athlete's personal thread. "+" and `/reset`
 * divide the one thread instead of starting another, and the coach reads the
 * thread from here on — so the athlete sees where it started over.
 */
export default function TopicDivider({ label, time }: { label: string; time: string }) {
  const text = time ? `${label} · ${time}` : label;
  return (
    <View
      className="my-4 flex-row items-center gap-3"
      accessible
      accessibilityLabel={text}
      testID="topic-divider"
    >
      <View className="h-px flex-1 bg-border-faint" />
      <Text className="text-xs font-medium text-on-surface-variant">{text}</Text>
      <View className="h-px flex-1 bg-border-faint" />
    </View>
  );
}
