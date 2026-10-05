// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Opens an external URL in the device's browser, and tells the athlete when the device cannot
// ABOUTME: Shared by About's rows and a verdict's studies, so a link the device cannot open says so in both

import { Alert, Linking } from 'react-native';

/**
 * Open an external URL, telling the athlete when the device cannot.
 *
 * A bare `Linking.openURL` rejects on a device with no handler, and an
 * unhandled rejection looks identical to the row doing nothing.
 */
export async function openExternal(
  url: string,
  t: (key: string, opts?: Record<string, unknown>) => string,
): Promise<void> {
  try {
    await Linking.openURL(url);
  } catch {
    Alert.alert(t('app.couldNotOpenLink'), t('app.openInBrowserInstead', { url }));
  }
}
