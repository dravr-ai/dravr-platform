// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Language switcher dropdown component for web frontend
// ABOUTME: Provides user-friendly language selection with flags and names

import { useLanguageSwitcher, SUPPORTED_LANGUAGES, LANGUAGE_NAMES, type SupportedLanguage } from '@pierre/i18n';
import { Select } from './ui';

const LANGUAGE_FLAGS: Record<SupportedLanguage, string> = {
  en: '🇺🇸',
  es: '🇪🇸',
  fr: '🇫🇷',
};

export function LanguageSwitcher() {
  const { currentLanguage, changeLanguage } = useLanguageSwitcher();

  return (
    <div className="inline-block w-44">
      <Select
        value={currentLanguage}
        onChange={(e) => changeLanguage(e.target.value as SupportedLanguage)}
        aria-label="Select language"
        options={SUPPORTED_LANGUAGES.map((lang) => ({
          value: lang,
          label: `${LANGUAGE_FLAGS[lang]} ${LANGUAGE_NAMES[lang]}`,
        }))}
      />
    </div>
  );
}
