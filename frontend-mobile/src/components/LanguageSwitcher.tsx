// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Language switcher rows for the mobile app, covering all five platform locales
// ABOUTME: Reports when the chrome moved but the coach's reply language could not be saved

import React from 'react';
import { View, Text, TouchableOpacity } from 'react-native';
import { useTranslation, SUPPORTED_LANGUAGES, LANGUAGE_NAMES, type SupportedLanguage } from '@pierre/i18n';
import { useLanguageSwitcherNative } from '@pierre/i18n/native';

const LANGUAGE_FLAGS: Record<SupportedLanguage, string> = {
  fr: '🇫🇷',
  en: '🇬🇧',
  es: '🇪🇸',
  de: '🇩🇪',
  pt: '🇵🇹',
};

/** Props for [`LanguageSwitcher`]. */
export interface LanguageSwitcherProps {
  /**
   * The locale stored on the signed-in user's account. Adopted on first
   * launch when this device has no stored choice of its own, so a language
   * picked on the web carries over to the phone.
   */
  serverLocale?: string;
}

/**
 * Pick the app language.
 *
 * The same choice sets the chrome language and `users.locale`, so the coach
 * answers in the language the athlete reads the app in.
 */
export function LanguageSwitcher({ serverLocale }: LanguageSwitcherProps) {
  const { t } = useTranslation();
  const { currentLanguage, changeLanguage, syncState } = useLanguageSwitcherNative({ serverLocale });

  return (
    <View testID="language-switcher">
      <View className="flex-row flex-wrap gap-2">
        {SUPPORTED_LANGUAGES.map((lang) => {
          const isSelected = currentLanguage === lang;
          return (
            <TouchableOpacity
              key={lang}
              testID={`language-option-${lang}`}
              accessibilityRole="radio"
              accessibilityState={{ selected: isSelected }}
              accessibilityLabel={LANGUAGE_NAMES[lang]}
              disabled={syncState === 'saving'}
              onPress={() => { void changeLanguage(lang); }}
              className={`py-2 px-3 rounded-xl items-center ${
                isSelected
                  ? 'bg-primary'
                  : 'bg-surface-container-high border border-pierre-gray-700'
              }`}
              activeOpacity={0.7}
            >
              <Text className="text-xl mb-1">{LANGUAGE_FLAGS[lang]}</Text>
              <Text
                className={`text-xs font-medium ${
                  isSelected ? 'text-on-surface' : 'text-pierre-gray-300'
                }`}
              >
                {LANGUAGE_NAMES[lang]}
              </Text>
            </TouchableOpacity>
          );
        })}
      </View>
      {syncState === 'saving' && (
        <Text className="mt-2 text-xs text-pierre-gray-300">{t('settings.languageSaving')}</Text>
      )}
      {syncState === 'error' && (
        <Text testID="language-sync-error" className="mt-2 text-xs text-error">
          {t('settings.languageSyncFailed')}
        </Text>
      )}
    </View>
  );
}
