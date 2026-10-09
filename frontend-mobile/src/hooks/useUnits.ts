// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mobile binding of the athlete's units hook — the shared React Query hook over this client's athlete API
// ABOUTME: The phone's own first locale (`en-US`) is the device locale the server falls back on (carnet#835)

import { getLocales } from 'expo-localization';
import { createUnitPreferencesHook } from '@pierre/ui-logic';
import { athleteApi } from '../services/api';

/** The phone's preferred locale, `en-US`; an empty string when it reports none. */
function deviceLocale(): string {
  return getLocales()[0]?.languageTag ?? '';
}

/**
 * The athlete's units and their Settings choice. The API is reached when a
 * read or write runs, so importing it touches no client.
 */
export const useUnitPreferences = createUnitPreferencesHook(
  {
    getUnitPreferences: (locale) => athleteApi.getUnitPreferences(locale),
    updateUnitPreferences: (update) => athleteApi.updateUnitPreferences(update),
  },
  deviceLocale,
);
