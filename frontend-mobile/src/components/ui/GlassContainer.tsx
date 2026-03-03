// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Reusable frosted glass container using expo-blur BlurView
// ABOUTME: Applies the Pierre design system glass styling with blur, border, and shadow

import React from 'react';
import { type ViewStyle, type StyleProp, View } from 'react-native';
import { BlurView } from 'expo-blur';
import { glassCard } from '../../constants/theme';

interface GlassContainerProps {
  children: React.ReactNode;
  style?: StyleProp<ViewStyle>;
  intensity?: number;
  borderRadius?: number;
  testID?: string;
}

export function GlassContainer({
  children,
  style,
  intensity = 40,
  borderRadius = 24,
  testID,
}: GlassContainerProps) {
  return (
    <View
      testID={testID}
      style={[
        {
          borderRadius,
          borderWidth: glassCard.borderWidth,
          borderColor: glassCard.borderColor,
          shadowColor: glassCard.shadowColor,
          shadowOffset: { width: 0, height: 4 },
          shadowOpacity: glassCard.shadowOpacity,
          shadowRadius: glassCard.shadowRadius,
          elevation: 8,
          overflow: 'hidden',
        },
        style,
      ]}
    >
      <BlurView
        intensity={intensity}
        tint="dark"
        style={{ flex: 1 }}
      >
        {children}
      </BlurView>
    </View>
  );
}
