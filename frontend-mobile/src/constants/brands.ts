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

