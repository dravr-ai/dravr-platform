// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Dismissible usage warning banner for the mobile chat interface
// ABOUTME: Draws the warning family for a warning or burst level and the error family for blocked, on theme tokens

import React, { useState } from 'react';
import { View, Text, TouchableOpacity } from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import type { WarningLevel } from '@pierre/chat-utils';
import { useThemeColors } from '../../constants/theme';
import { ALERT_TONE, alertInkColor, type AlertTone } from '../../components/ui';
import { useTranslation } from '@pierre/i18n';

interface UsageWarningBannerProps {
  /** The warning severity level */
  level: WarningLevel;
  /** The message to display */
  message: string;
}

/**
 * Each level draws through the alert table (DESIGN.md "Alerts"), the web
 * banner's three steps: warning is the warning alert, burst keeps that tone
 * and deepens its edge so the escalation stays visible, blocked switches to
 * the error alert. The icon takes the tone's ink — the bare warning hue on its
 * own tint is under the 3:1 a glyph needs in light.
 */
const LEVEL_CONFIG: Record<Exclude<WarningLevel, 'none'>, {
  tone: AlertTone;
  edgeClass: string;
  iconName: 'alert-circle' | 'warning' | 'close-circle';
}> = {
  warning: {
    tone: 'warning',
    edgeClass: ALERT_TONE.warning.edge,
    iconName: 'alert-circle',
  },
  burst: {
    tone: 'warning',
    edgeClass: 'border-warning/60',
    iconName: 'warning',
  },
  blocked: {
    tone: 'error',
    edgeClass: ALERT_TONE.error.edge,
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
  const { fill, ink } = ALERT_TONE[config.tone];
  const iconColor = alertInkColor(config.tone, colors);

  return (
    <View
      className={`flex-row items-center px-4 py-2.5 ${fill} border-b ${config.edgeClass}`}
      accessibilityRole="alert"
      accessibilityLiveRegion="polite"
      testID="usage-warning-banner"
    >
      <Ionicons name={config.iconName} size={16} color={iconColor} />
      <Text className={`flex-1 ml-2 text-xs ${ink}`}>{message}</Text>
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
