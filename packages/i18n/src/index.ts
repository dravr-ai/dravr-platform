// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Main export file for @pierre/i18n package
// ABOUTME: Provides unified API for web and mobile i18n functionality

export {
  initI18n,
  i18n,
  defaultI18nConfig,
  isSupportedLanguage,
  SUPPORTED_LANGUAGES,
  LANGUAGE_NAMES,
  DEFAULT_LANGUAGE,
} from './config';
export type { SupportedLanguage, I18nInitOptions } from './config';
export { registerLocalePersister, persistLocaleToServer, LocalePersisterMissingError } from './localeSync';
export type { LocalePersister } from './localeSync';
export { applyLiveBundle, installLiveOverlay, nestDotted } from './liveBundle';
export type { BundleFetcher, LiveOverlay } from './liveBundle';
export { LANGUAGE_STORAGE_KEY } from './switcherCore';
export type {
  LanguageSwitcherOptions,
  LanguageSwitcherResult,
  LocaleSyncState,
  LocaleStorage,
} from './switcherCore';
export { useTranslation } from './types';
export type { TranslationKeys, TranslationHandle, TFunction } from './types';
export { useLanguageSwitcher } from './useLanguageSwitcher';

// Re-export core i18next types for convenience
export type { i18n as I18nInstance, TOptions } from 'i18next';
export { I18nextProvider } from 'react-i18next';
