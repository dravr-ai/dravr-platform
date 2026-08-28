// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proper nouns the UI renders verbatim — provider brands and persona names
// ABOUTME: Data, not copy: identical in all five locales, and never routed through t()

/**
 * How each connectable service writes its own name.
 *
 * These are trademarks. They do not get translated, they do not vary by
 * locale, and putting them in the translation corpus would invite exactly
 * that — a translator seeing "Apple" in a list of strings has no way to know
 * it is a company rather than the fruit.
 *
 * They lived inline in SciotteLoginModal, OnboardingConnectScreen and
 * ConnectionsScreen, which is three copies of the same fact and three places
 * for a rename to be missed.
 */
export const PROVIDER_BRAND = {
  strava: 'Strava',
  garmin: 'Garmin',
  garminConnect: 'Garmin Connect',
  google: 'Google',
  apple: 'Apple',
  whoop: 'WHOOP',
  intervalsIcu: 'Intervals.icu',
  fitbit: 'Fitbit',
} as const;

/**
 * The display name of each coaching persona.
 *
 * Deliberately untranslated: this string is stored on the account, quoted back
 * inside the coach's own system prompt, and shown in the settings list. If the
 * settings list said "Décontracté" while the stored value was "Casual", the two
 * would stop matching and nobody would be able to tell which persona was
 * active. The tagline and description beside it ARE translated.
 */
export const PERSONA_NAME = {
  casual: 'Casual',
  enthusiast: 'Enthusiast',
  power_athlete: 'Power-athlete',
  coach: 'Coach',
} as const;
