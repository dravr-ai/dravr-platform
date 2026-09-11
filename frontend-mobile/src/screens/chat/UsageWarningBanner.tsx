// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Dismissible usage warning banner for the mobile chat interface
// ABOUTME: Draws the warning family for a warning or burst level and the error family for blocked, on theme tokens

import React, { useState } from 'react';
import { View, Text, TouchableOpacity } from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import type { WarningLevel } from './useUsageStatus';
import { useThemeColors } from '../../constants/theme';
import { useTranslation } from '@pierre/i18n';

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
  /** Which palette hue the icon takes: the warning ink or the error ink. */
  iconTone: 'warning' | 'error';
  iconName: 'alert-circle' | 'warning' | 'close-circle';
}> = {
  warning: {
    bgClass: 'bg-warning/15',
    borderClass: 'border-warning/30',
    textClass: 'text-on-warning-container',
    iconTone: 'warning',
    iconName: 'alert-circle',
  },
  burst: {
    bgClass: 'bg-warning/15',
    borderClass: 'border-warning/30',
    textClass: 'text-on-warning-container',
    iconTone: 'warning',
    iconName: 'warning',
  },
  blocked: {
    bgClass: 'bg-error-container',
    borderClass: 'border-error/30',
    textClass: 'text-on-error-container',
    iconTone: 'error',
    iconName: 'close-circle',
  },
};

export function UsageWarningBanner({ level, message }: UsageWarningBannerProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const [dismissed, setDismissed] = useState(false);

  if (level === 'none' || dismissed || !message) {
    return null;
  }

  const config = LEVEL_CONFIG[level];
  const iconColor = config.iconTone === 'warning' ? colors.warning : colors.error;

  return (
    <View
      className={`flex-row items-center px-4 py-2.5 ${config.bgClass} border-b ${config.borderClass}`}
      accessibilityRole="alert"
      accessibilityLiveRegion="polite"
      testID="usage-warning-banner"
    >
      <Ionicons name={config.iconName} size={16} color={iconColor} />
      <Text className={`flex-1 ml-2 text-xs ${config.textClass}`}>{message}</Text>
      {level !== 'blocked' && (
        <TouchableOpacity
          onPress={() => setDismissed(true)}
          className="p-1"
          accessibilityLabel={t('app.dismissWarning')}
          testID="dismiss-warning-button"
        >
          <Ionicons name="close" size={14} color={iconColor} />
        </TouchableOpacity>
      )}
    </View>
  );
}
