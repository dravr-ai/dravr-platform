// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The "Import your own data" row under the provider list: one ink line, no card, opens the import screen
// ABOUTME: Shown in Connections and in onboarding's connect step, under the providers it does not replace

import React from 'react';
import { Pressable, Text, View } from 'react-native';
import { useRouter } from 'expo-router';
import { useTranslation } from '@pierre/i18n';
import { IMPORT_ROUTE, ONBOARDING_IMPORT_ROUTE } from '../navigation/routes';

interface ImportOwnDataRowProps {
  /** Open the onboarding twin of the screen, inside the connect step. */
  onboarding?: boolean;
}

export function ImportOwnDataRow({ onboarding = false }: ImportOwnDataRowProps) {
  const { t } = useTranslation();
  const router = useRouter();
  return (
    <Pressable
      onPress={() => router.push(onboarding ? ONBOARDING_IMPORT_ROUTE : IMPORT_ROUTE)}
      accessibilityRole="button"
      accessibilityLabel={t('providers.import.link')}
      testID="import-own-data-link"
      className="flex-row items-center py-3 px-4"
    >
      <View className="flex-1">
        <Text className="text-base font-medium text-text-primary">{t('providers.import.link')} →</Text>
        <Text className="text-xs text-text-secondary">{t('providers.import.linkHint')}</Text>
      </View>
    </Pressable>
  );
}
