// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Opening the import-your-own-data window: a new window in a browser, the same one in the installed app
// ABOUTME: A blocked popup falls back to the same tab; the import window lists the sources an athlete can import

import { isInstalled } from '../../hooks/useInstallPrompt';

/** Where the import window lives: a path the SPA renders, never proxied to the backend. */
export const IMPORT_PATH = '/import';

/**
 * Open the import window.
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
