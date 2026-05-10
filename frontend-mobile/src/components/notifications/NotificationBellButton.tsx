// ABOUTME: Bell icon button with unread badge for notification center navigation
// ABOUTME: Displays unread count and navigates to the notification center screen

import React from 'react';
import { View, TouchableOpacity, Text } from 'react-native';
import { useRouter } from 'expo-router';
import { Bell } from 'lucide-react-native';
import { useUnreadCount } from '../../hooks/useNotifications';
import { useThemeColors } from '../../constants/theme';

interface NotificationBellButtonProps {
  /** Icon size in pixels */
  size?: number;
  /** Icon color override; falls back to the active theme's secondary text. */
  color?: string;
}

/**
 * Bell icon button that shows unread notification count as a badge.
 * Navigates to the notification center when pressed.
 */
export function NotificationBellButton({
  size = 22,
  color,
}: NotificationBellButtonProps) {
  const router = useRouter();
  const colors = useThemeColors();
  const { unreadCount } = useUnreadCount();
  const iconColor = color ?? colors.text.secondary;

  return (
    <TouchableOpacity
      className="w-10 h-10 items-center justify-center"
      onPress={() => router.push('/(app)/notifications' as never)}
      testID="notification-bell"
      hitSlop={{ top: 8, bottom: 8, left: 8, right: 8 }}
    >
      <Bell size={size} color={iconColor} />
      {unreadCount > 0 && (
        <View
          className="absolute -top-0.5 -right-0.5 min-w-[18px] h-[18px] rounded-full items-center justify-center px-1"
          style={{ backgroundColor: colors.pierre.violet }}
        >
          <Text
            className="text-[10px] font-bold"
            style={{ color: colors.tokens.onPrimary }}
          >
            {unreadCount > 99 ? '99+' : unreadCount}
          </Text>
        </View>
      )}
    </TouchableOpacity>
  );
}
