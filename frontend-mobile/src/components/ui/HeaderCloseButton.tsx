// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The "Close" text button a modal carries in its native header, where a pushed screen has a back chevron
// ABOUTME: Dismisses the screen it is on; falls back to the chat list when nothing is beneath it

import React from 'react';
import { Text, TouchableOpacity } from 'react-native';
import { useRouter } from 'expo-router';
import { useTranslation } from '@pierre/i18n';
import { useThemeColors } from '../../constants/theme';
import { CHAT_LIST_ROUTE } from '../../navigation/routes';

interface HeaderCloseButtonProps {
  testID?: string;
}

export function HeaderCloseButton({ testID = 'back-button' }: HeaderCloseButtonProps) {
  const { t } = useTranslation();
  const router = useRouter();
  const colors = useThemeColors();

  const close = () => {
    if (router.canGoBack()) {
      router.back();
    } else {
      router.replace(CHAT_LIST_ROUTE);
    }
  };

  return (
    <TouchableOpacity
      onPress={close}
      hitSlop={{ top: 8, bottom: 8, left: 8, right: 8 }}
      accessibilityRole="button"
      accessibilityLabel={t('common.close')}
      testID={testID}
    >
      <Text className="text-lg" style={{ color: colors.tokens.primary }}>{t('common.close')}</Text>
    </TouchableOpacity>
  );
}
