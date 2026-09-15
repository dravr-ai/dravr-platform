// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Language switcher for the mobile app — one settings Row per platform locale, a check on the chosen one
// ABOUTME: One tap moves both the chrome language and the account locale the coach answers in

import React from 'react';
import { View, Text } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { useTranslation, SUPPORTED_LANGUAGES, LANGUAGE_NAMES } from '@pierre/i18n';
import { useLanguageSwitcherNative } from '@pierre/i18n/native';
import { useThemeColors } from '../constants/theme';
import { Row } from './ui/Row';

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
 * answers in the language the athlete reads the app in. Each locale is a row
 * named in its own language, with the one green check on the row in force:
 * a radio group read as a list, not a grid of flag tiles (DESIGN.md §10).
 */
export function LanguageSwitcher({ serverLocale }: LanguageSwitcherProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const { currentLanguage, changeLanguage, syncState } = useLanguageSwitcherNative({ serverLocale });

  return (
    <View testID="language-switcher">
      {SUPPORTED_LANGUAGES.map((lang, index) => {
        const isSelected = currentLanguage === lang;
        return (
          <Row
            key={lang}
            title={LANGUAGE_NAMES[lang]}
            onPress={() => {
              if (syncState !== 'saving') void changeLanguage(lang);
            }}
            showChevron={false}
            last={index === SUPPORTED_LANGUAGES.length - 1}
            accessibilityRole="radio"
            accessibilityState={{ selected: isSelected }}
            accessibilityLabel={LANGUAGE_NAMES[lang]}
            trailing={isSelected ? <Feather name="check" size={18} color={colors.tokens.primary} /> : undefined}
            testID={`language-option-${lang}`}
          />
        );
      })}
      {syncState === 'saving' && (
        <Text className="mt-2 px-4 text-xs text-text-secondary">{t('settings.languageSaving')}</Text>
      )}
      {syncState === 'error' && (
        <Text testID="language-sync-error" className="mt-2 px-4 text-xs text-error">
          {t('settings.languageSyncFailed')}
        </Text>
      )}
    </View>
  );
}
