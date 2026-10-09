// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The web binding of the athlete's units hook — the shared React Query hook over this client's athlete API
// ABOUTME: The browser's own locale (`navigator.language`) is the device locale the server falls back on (carnet#835)

import { createUnitPreferencesHook } from '@pierre/ui-logic';
import { athleteApi } from '../services/api';

/** The locale the browser reports, `en-US`; an empty string when it reports none. */
function browserLocale(): string {
  return typeof navigator === 'undefined' ? '' : navigator.language;
}

/**
 * The athlete's units and their Settings choice. The API is reached when a
 * read or write runs, so importing it touches no client.
 */
export const useUnitPreferences = createUnitPreferencesHook(
  {
    getUnitPreferences: (deviceLocale) => athleteApi.getUnitPreferences(deviceLocale),
    updateUnitPreferences: (update) => athleteApi.updateUnitPreferences(update),
  },
  browserLocale,
);
