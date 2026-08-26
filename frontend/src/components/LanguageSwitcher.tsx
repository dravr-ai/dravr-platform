// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Language switcher dropdown for the web app, covering all five platform locales
// ABOUTME: Reports when the chrome moved but the coach's reply language could not be saved

import { useTranslation, useLanguageSwitcher, SUPPORTED_LANGUAGES, LANGUAGE_NAMES, type SupportedLanguage } from '@pierre/i18n';
import { Select } from './ui';

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
   * The locale stored on the signed-in user's account. Adopted on first load
   * when this browser has no stored choice of its own, so a language picked
   * on another device carries over instead of resetting to the default.
   */
  serverLocale?: string;
}

/**
 * Pick the app language.
 *
 * The same choice sets the chrome language and `users.locale`, so the coach
 * answers in the language the user reads the app in.
 */
export function LanguageSwitcher({ serverLocale }: LanguageSwitcherProps) {
  const { t } = useTranslation();
  const { currentLanguage, changeLanguage, syncState } = useLanguageSwitcher({ serverLocale });

  return (
    <div className="inline-block w-44">
      <Select
        value={currentLanguage}
        onChange={(e) => { void changeLanguage(e.target.value as SupportedLanguage); }}
        aria-label={t('settings.selectLanguage')}
        disabled={syncState === 'saving'}
        options={SUPPORTED_LANGUAGES.map((lang) => ({
          value: lang,
          label: `${LANGUAGE_FLAGS[lang]} ${LANGUAGE_NAMES[lang]}`,
        }))}
      />
      {syncState === 'saving' && (
        <p className="mt-1 text-xs text-on-surface-variant">{t('settings.languageSaving')}</p>
      )}
      {syncState === 'error' && (
        <p role="alert" className="mt-1 text-xs text-error">{t('settings.languageSyncFailed')}</p>
      )}
    </div>
  );
}
