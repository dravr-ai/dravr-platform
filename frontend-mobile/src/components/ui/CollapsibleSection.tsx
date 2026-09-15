// ABOUTME: A Section with a disclosure — the 13 / 600 title and a chevron on one pressable header, the content under it while expanded
// ABOUTME: No fill, no border, no radius: sections are separated by space (32 below each), never by a box (DESIGN.md §10)

import React, { useState, useCallback } from 'react';
import { View, Text, Pressable } from 'react-native';
import Animated, { FadeIn, FadeOut } from 'react-native-reanimated';
import { Feather } from '@expo/vector-icons';
import { useThemeColors } from '../../constants/theme';

interface CollapsibleSectionProps {
  title: string;
  defaultExpanded?: boolean;
  children: React.ReactNode;
  testID?: string;
}

export function CollapsibleSection({
  title,
  defaultExpanded = false,
  children,
  testID,
}: CollapsibleSectionProps) {
  const colors = useThemeColors();
  const [expanded, setExpanded] = useState(defaultExpanded);

  const toggle = useCallback(() => {
    setExpanded((prev) => !prev);
  }, []);

  return (
    <View className="mb-8" testID={testID}>
      <Pressable
        className="flex-row items-center justify-between py-2"
        onPress={toggle}
        accessibilityRole="button"
        accessibilityState={{ expanded }}
        testID={testID ? `${testID}-toggle` : undefined}
      >
        <Text className="text-sm font-semibold text-text-primary">{title}</Text>
        <Feather
          name={expanded ? 'chevron-up' : 'chevron-down'}
          size={18}
          color={colors.text.secondary}
        />
      </Pressable>

      {expanded && (
        <Animated.View
          entering={FadeIn.duration(200)}
          exiting={FadeOut.duration(150)}
          className="pt-2"
          testID={testID ? `${testID}-content` : undefined}
        >
          {children}
        </Animated.View>
      )}
    </View>
  );
}
