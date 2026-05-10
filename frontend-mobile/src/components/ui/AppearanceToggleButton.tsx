// ABOUTME: Quick light/dark toggle for placement in screen headers
// ABOUTME: Single tap flips the persisted appearance preference between Light and Dark

import React from 'react';
import { TouchableOpacity } from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import { useTheme, useThemeColors } from '../../constants/theme';

interface AppearanceToggleButtonProps {
  /** Icon size in pixels. Defaults to 20 to match the notification bell. */
  size?: number;
  /** Optional explicit color override; falls back to the active theme's secondary text. */
  color?: string;
  testID?: string;
}

/**
 * Sun / moon toggle for the screen header. Tapping flips the persisted
 * AppearancePref between 'light' and 'dark'. The full system/light/dark
 * picker remains in Settings → Appearance for users who want to follow OS.
 */
export function AppearanceToggleButton({
  size = 20,
  color,
  testID,
}: AppearanceToggleButtonProps) {
  const { scheme, setPref } = useTheme();
  const colors = useThemeColors();
  const isDark = scheme === 'dark';
  const iconColor = color ?? colors.text.secondary;

  const handlePress = () => {
    void setPref(isDark ? 'light' : 'dark');
  };

  return (
    <TouchableOpacity
      className="w-10 h-10 items-center justify-center"
      onPress={handlePress}
      hitSlop={{ top: 8, bottom: 8, left: 8, right: 8 }}
      accessibilityRole="button"
      accessibilityLabel={isDark ? 'Switch to light mode' : 'Switch to dark mode'}
      testID={testID ?? 'appearance-toggle-button'}
    >
      <Ionicons
        name={isDark ? 'sunny-outline' : 'moon-outline'}
        size={size}
        color={iconColor}
      />
    </TouchableOpacity>
  );
}
