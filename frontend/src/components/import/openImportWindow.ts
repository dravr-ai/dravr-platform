// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Opening the import-your-own-data window: a new window in a browser, the same one in the installed app
// ABOUTME: Names each source's import by its hash, so a link (a provider card's export link) opens that import directly

import {
  APPLE_HEALTH_ARCHIVE_SOURCE,
  COROS_ARCHIVE_SOURCE,
  GARMIN_ARCHIVE_SOURCE,
  STRAVA_ARCHIVE_SOURCE,
  WHOOP_ARCHIVE_SOURCE,
} from '@pierre/shared-types';
import { isInstalled } from '../../hooks/useInstallPrompt';

/** Where the import window lives: a path the SPA renders, never proxied to the backend. */
export const IMPORT_PATH = '/import';

/** The hash a link into the window names each source's import by. */
const SOURCE_HASHES: Readonly<Record<string, string>> = {
  [STRAVA_ARCHIVE_SOURCE]: '#strava',
  [GARMIN_ARCHIVE_SOURCE]: '#garmin',
  [APPLE_HEALTH_ARCHIVE_SOURCE]: '#apple',
  [COROS_ARCHIVE_SOURCE]: '#coros',
  [WHOOP_ARCHIVE_SOURCE]: '#whoop',
};

/** The hash that opens the import of `source`, or `''` (the source list) for a source the window has no hash for. */
export function importWindowHash(source: string): string {
  return SOURCE_HASHES[source] ?? '';
}

/** The source whose import `hash` opens, or `null` for the source list. */
export function importSourceForHash(hash: string): string | null {
  return Object.keys(SOURCE_HASHES).find((source) => SOURCE_HASHES[source] === hash) ?? null;
}

/**
 * Open the import window, on the import `hash` names (`importWindowHash`), or
 * on the source list.
 *
 * In a browser it opens a window of its own, so the provider list it was
 * opened from stays where it was. The installed app navigates in place: on
 * iOS a new window would open Safari, outside the app's session. A popup the
 * browser blocks falls back to the same tab.
 */
export function openImportWindow(hash = ''): void {
  const target = `${IMPORT_PATH}${hash}`;
  if (isInstalled()) {
    window.location.assign(target);
    return;
  }
  if (!window.open(target, '_blank')) {
    window.location.assign(target);
  }
}
