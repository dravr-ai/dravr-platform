// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One action row of the tab bar's "+" sheet — icon, label, haptic and staggered entrance
// ABOUTME: Its height is fixed and exported, so the sheet can size itself from the rows it holds

import React from 'react';
import { Pressable, Text } from 'react-native';
import Animated, { FadeInDown } from 'react-native-reanimated';
import * as Haptics from 'expo-haptics';
import type { LucideIcon } from 'lucide-react-native';
import { useThemeColors } from '../../constants/theme';

/**
 * The height of one row, in points.
 *
 * Stated rather than intrinsic: the sheet's height is the sum of its rows, so
 * a row that measured itself would leave the sheet guessing at how tall to
 * open. Twelve points of padding above and below a 24pt line.
 */
export const MENU_ROW_HEIGHT = 48;

interface TabMenuItemProps {
  icon: LucideIcon;
  label: string;
  delay: number;
  onPress: () => void;
  testID?: string;
}

export function TabMenuItem({
  icon: Icon,
  label,
  delay,
  onPress,
  testID,
}: TabMenuItemProps) {
  const colors = useThemeColors();
  const handlePress = () => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    onPress();
  };

  return (
    <Animated.View entering={FadeInDown.delay(delay).duration(100)}>
      <Pressable
        testID={testID}
        onPress={handlePress}
        className="flex-row items-center px-4 rounded-xl"
        style={{ height: MENU_ROW_HEIGHT }}
      >
        <Icon size={20} color={colors.pierre.violet} />
        <Text
          className="ml-3 text-base font-medium"
          style={{ color: colors.text.secondary }}
        >
          {label}
        </Text>
      </Pressable>
    </Animated.View>
  );
}
