// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Individual row item for the expandable tab bar menu
// ABOUTME: Supports active state, staggered enter animation, and haptic feedback

import React from 'react';
import { Pressable, Text } from 'react-native';
import Animated, { FadeInDown } from 'react-native-reanimated';
import * as Haptics from 'expo-haptics';
import type { LucideIcon } from 'lucide-react-native';
import { useThemeColors } from '../../constants/theme';

interface TabMenuItemProps {
  icon: LucideIcon;
  label: string;
  isActive: boolean;
  delay: number;
  onPress: () => void;
  isQuickAction?: boolean;
  testID?: string;
}

export function TabMenuItem({
  icon: Icon,
  label,
  isActive,
  delay,
  onPress,
  isQuickAction = false,
  testID,
}: TabMenuItemProps) {
  const colors = useThemeColors();
  const handlePress = () => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    onPress();
  };

  const iconColor = isQuickAction
    ? colors.pierre.violet
    : isActive
      ? colors.pierre.violet
      : colors.text.secondary;

  const textColor = isActive ? colors.pierre.violet : colors.text.secondary;

  return (
    <Animated.View entering={FadeInDown.delay(delay).duration(100)}>
      <Pressable
        testID={testID}
        onPress={handlePress}
        className="flex-row items-center px-4 py-3 rounded-xl"
        style={
          isActive
            ? { backgroundColor: 'rgba(139, 92, 246, 0.15)' }
            : undefined
        }
      >
        <Icon size={20} color={iconColor} />
        <Text
          className="ml-3 text-base font-medium"
          style={{ color: textColor }}
        >
          {label}
        </Text>
      </Pressable>
    </Animated.View>
  );
}
