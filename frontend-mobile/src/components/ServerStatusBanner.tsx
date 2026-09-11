// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
// ABOUTME: Banner displayed when Pierre server is unreachable
// ABOUTME: Shows a compact error-tinted bar at the top of the app with server status

import React from 'react';
import { View, Text, TouchableOpacity } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { useThemeColors } from '../constants/theme';
import { useTranslation } from '@pierre/i18n';

interface ServerStatusBannerProps {
  onRetry: () => void;
  isChecking: boolean;
}

export function ServerStatusBanner({ onRetry, isChecking }: ServerStatusBannerProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const insets = useSafeAreaInsets();

  return (
    <View
      className="bg-error-container px-4 py-2 flex-row items-center justify-between"
      style={{ paddingTop: insets.top + 4 }}
    >
      <View className="flex-row items-center flex-1 mr-3">
        <Feather name="wifi-off" size={14} color={colors.tokens.onErrorContainer} />
        <Text className="text-on-error-container text-sm font-medium ml-2">
          {t('app.serverUnreachable')}
        </Text>
      </View>
      <TouchableOpacity
        onPress={onRetry}
        disabled={isChecking}
        className="bg-error px-3 py-1 rounded-md"
        accessibilityLabel={t('app.retryServerConnection')}
      >
        <Text className="text-on-error text-xs font-medium">
          {isChecking ? t('app.checking') : t('common.retry')}
        </Text>
      </TouchableOpacity>
    </View>
  );
}
