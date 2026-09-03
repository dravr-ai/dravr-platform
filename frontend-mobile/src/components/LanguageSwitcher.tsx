// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Language switcher tiles for the mobile app, covering all five platform locales
// ABOUTME: Lays the tiles on explicit rows so the last line never strands one locale alone

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

/**
 * How many tiles a line holds without stranding one on its own.
 *
 * Five locales in a plain wrapping row laid out 4 + 1 on a phone, leaving
 * Portuguese alone beneath four siblings. Deciding the width here instead of
 * letting the wrapper decide keeps the last line populated for any count:
 * the widest row that leaves a remainder of anything but one.
 */
export function languageGridColumns(count: number, max = 4): number {
  for (let columns = Math.min(max, count); columns > 1; columns -= 1) {
    const remainder = count % columns;
    if (remainder === 0 || remainder > 1) {
      return columns;
    }
  }
  return Math.max(1, count);
}

/** Split `items` into lines of at most `columns` entries, in order. */
export function languageGridRows<T>(items: readonly T[], columns: number): T[][] {
  const rows: T[][] = [];
  for (let i = 0; i < items.length; i += columns) {
    rows.push(items.slice(i, i + columns));
  }
  return rows;
}

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

  const columns = languageGridColumns(SUPPORTED_LANGUAGES.length);
  const rows = languageGridRows(SUPPORTED_LANGUAGES, columns);

  return (
    <View testID="language-switcher">
      {rows.map((row, rowIndex) => (
        <View key={row.join('-')} className="flex-row gap-2 mb-2" testID={`language-row-${rowIndex}`}>
          {row.map((lang) => {
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
                style={{ flex: 1 }}
                className={`py-2 px-2 rounded-xl items-center ${
                  isSelected
                    ? 'bg-primary'
                    : 'bg-surface-container-high border border-pierre-gray-700'
                }`}
                activeOpacity={0.7}
              >
                <Text className="text-xl mb-1">{LANGUAGE_FLAGS[lang]}</Text>
                <Text
                  numberOfLines={1}
                  className={`text-xs font-medium ${
                    isSelected ? 'text-on-surface' : 'text-pierre-gray-300'
                  }`}
                >
                  {LANGUAGE_NAMES[lang]}
                </Text>
              </TouchableOpacity>
            );
          })}
          {/* A short last line keeps its tiles the same width as a full one
              instead of stretching them across the row. */}
          {[...Array(columns - row.length).keys()].map((index) => (
            <View key={`filler-${index}`} style={{ flex: 1 }} />
          ))}
        </View>
      ))}
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
