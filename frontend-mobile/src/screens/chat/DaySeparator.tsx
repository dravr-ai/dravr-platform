// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The pill between two days of a thread — today, yesterday, or the date spelled out
// ABOUTME: The React Native half of the web separator, drawn from the same dayLabelFor decision

import React from 'react';
import { View, Text } from 'react-native';

export default function DaySeparator({ label }: { label: string }) {
  return (
    <View className="my-3 flex-row justify-center" testID="day-separator">
      <Text
        accessibilityRole="header"
        className="rounded-full bg-surface-container-high px-3 py-1 text-xs text-on-surface-variant"
      >
        {label}
      </Text>
    </View>
  );
}
