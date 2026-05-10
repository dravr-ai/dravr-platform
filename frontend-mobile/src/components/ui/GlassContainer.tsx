// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Reusable frosted glass container using expo-blur BlurView
// ABOUTME: Applies the Pierre design system glass styling with blur, border, and shadow

import React from 'react';
import { type ViewStyle, type StyleProp, View } from 'react-native';
import { BlurView } from 'expo-blur';
import { useTheme } from '../../contexts/ThemeContext';

interface GlassContainerProps {
  children: React.ReactNode;
  style?: StyleProp<ViewStyle>;
  intensity?: number;
  borderRadius?: number;
  /**
   * Override the blur tint. When omitted the tint follows the active theme
   * scheme — `systemThinMaterialDark` on dark, `systemThinMaterialLight` on
   * light — which gives a clean iOS-native frosted look in both palettes.
   */
  tint?: 'light' | 'dark' | 'default' | 'extraLight' | 'regular' | 'prominent';
  testID?: string;
}

export function GlassContainer({
  children,
  style,
  intensity = 40,
  borderRadius = 24,
  tint,
  testID,
}: GlassContainerProps) {
  const { scheme } = useTheme();
  const resolvedTint = tint ?? (scheme === 'dark' ? 'dark' : 'light');

  // Light surfaces benefit from a hairline outline-variant edge + a soft 4%
  // ink shadow. Dark surfaces use a higher-opacity black so cards still float
  // off the near-black canvas.
  const isDark = scheme === 'dark';
  const borderColor = isDark
    ? 'rgba(192, 200, 195, 0.18)'
    : 'rgba(26, 28, 27, 0.08)';
  const shadowColor = isDark ? '#000000' : '#1a1c1b';
  const shadowOpacity = isDark ? 0.45 : 0.06;

  return (
    <View
      testID={testID}
      style={[
        {
          borderRadius,
          borderWidth: 1,
          borderColor,
          shadowColor,
          shadowOffset: { width: 0, height: 8 },
          shadowOpacity,
          shadowRadius: 24,
          elevation: 8,
          overflow: 'hidden',
        },
        style,
      ]}
    >
      <BlurView
        intensity={intensity}
        tint={resolvedTint}
        style={{ flex: 1 }}
      >
        {children}
      </BlurView>
    </View>
  );
}
