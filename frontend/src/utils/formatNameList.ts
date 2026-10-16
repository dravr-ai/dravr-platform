// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Names as one phrase in the app's language — "Garmin and Strava", "Garmin et Strava"
// ABOUTME: Web only: Intl.ListFormat is a browser API the phone runtime does not carry

/** Names as one phrase in the app's language — "Garmin and Strava", "Garmin et Strava". */
export function formatNameList(names: readonly string[], language: string): string {
  return new Intl.ListFormat(language, { style: 'long', type: 'conjunction' }).format(names);
}
