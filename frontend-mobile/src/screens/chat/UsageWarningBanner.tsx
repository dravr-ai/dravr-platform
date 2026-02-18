// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Dismissible usage warning banner for the mobile chat interface
// ABOUTME: Shows yellow/orange/red warnings based on quota usage with NativeWind styling

import React, { useState } from 'react';
import { View, Text, TouchableOpacity } from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import type { WarningLevel } from './useUsageStatus';

interface UsageWarningBannerProps {
  /** The warning severity level */
  level: WarningLevel;
  /** The message to display */
  message: string;
}

const LEVEL_CONFIG: Record<Exclude<WarningLevel, 'none'>, {
  bgClass: string;
  borderClass: string;
  textClass: string;
  iconColor: string;
  iconName: 'alert-circle' | 'warning' | 'close-circle';
}> = {
  warning: {
    bgClass: 'bg-yellow-500/10',
    borderClass: 'border-yellow-500/30',
    textClass: 'text-yellow-300',
    iconColor: '#fbbf24',
    iconName: 'alert-circle',
  },
  burst: {
    bgClass: 'bg-orange-500/10',
    borderClass: 'border-orange-500/30',
    textClass: 'text-orange-300',
    iconColor: '#fb923c',
    iconName: 'warning',
  },
  blocked: {
    bgClass: 'bg-red-500/10',
    borderClass: 'border-red-500/30',
    textClass: 'text-red-400',
    iconColor: '#f87171',
    iconName: 'close-circle',
  },
};

export function UsageWarningBanner({ level, message }: UsageWarningBannerProps) {
  const [dismissed, setDismissed] = useState(false);

  if (level === 'none' || dismissed || !message) {
    return null;
  }

  const config = LEVEL_CONFIG[level];

  return (
    <View
      className={`flex-row items-center px-4 py-2.5 ${config.bgClass} border-b ${config.borderClass}`}
      accessibilityRole="alert"
      accessibilityLiveRegion="polite"
      testID="usage-warning-banner"
    >
      <Ionicons name={config.iconName} size={16} color={config.iconColor} />
      <Text className={`flex-1 ml-2 text-xs ${config.textClass}`}>{message}</Text>
      {level !== 'blocked' && (
        <TouchableOpacity
          onPress={() => setDismissed(true)}
          className="p-1"
          accessibilityLabel="Dismiss warning"
          testID="dismiss-warning-button"
        >
          <Ionicons name="close" size={14} color={config.iconColor} />
        </TouchableOpacity>
      )}
    </View>
  );
}
