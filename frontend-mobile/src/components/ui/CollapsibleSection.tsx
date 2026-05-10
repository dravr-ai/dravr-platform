// ABOUTME: Reusable collapsible accordion section with animated chevron
// ABOUTME: Glass card styling with smooth expand/collapse transitions on UI thread

import React, { useState, useCallback } from 'react';
import { View, Text, TouchableOpacity } from 'react-native';
import Animated, { FadeIn, FadeOut } from 'react-native-reanimated';
import { Feather } from '@expo/vector-icons';
import { glassCard, useThemeColors } from '../../constants/theme';

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
    <View
      className="mb-5 overflow-hidden"
      style={{
        ...glassCard,
        borderRadius: 12,
      }}
      testID={testID}
    >
      <TouchableOpacity
        className="flex-row items-center justify-between p-3.5"
        onPress={toggle}
        activeOpacity={0.7}
        testID={testID ? `${testID}-toggle` : undefined}
      >
        <Text className="text-text-primary text-sm font-semibold">{title}</Text>
        <Feather
          name={expanded ? 'chevron-up' : 'chevron-down'}
          size={18}
          color={colors.text.secondary}
        />
      </TouchableOpacity>

      {expanded && (
        <Animated.View
          entering={FadeIn.duration(200)}
          exiting={FadeOut.duration(150)}
          className="px-3.5 pb-3.5"
          testID={testID ? `${testID}-content` : undefined}
        >
          {children}
        </Animated.View>
      )}
    </View>
  );
}
