// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The app's one outbound-link opener — hands a URL to the device and tells the athlete when it cannot
// ABOUTME: Every screen opens links through it; eslint.config.js refuses a bare Linking.openURL anywhere else

import { Alert, Linking } from 'react-native';

/** What the athlete reads when the device cannot open the link. */
export interface OpenExternalFailure {
  title: string;
  message: string;
}

/**
 * Open an external URL, telling the athlete when the device cannot.
 *
 * A bare `Linking.openURL` rejects on a device with no handler (no browser
 * profile, no mail app for a `mailto:`), and an unhandled rejection looks
 * identical to the button doing nothing. This is therefore the only place the
 * app calls it: the mobile lint config refuses `openURL` and
 * `openBrowserAsync` in every other file.
 *
 * `failure` replaces the default alert where the screen can say something
 * more useful, such as which portal to open by hand.
 */
export async function openExternal(
  url: string,
  t: (key: string, opts?: Record<string, unknown>) => string,
  failure?: OpenExternalFailure,
): Promise<void> {
  try {
    await Linking.openURL(url);
  } catch {
    if (failure) {
      Alert.alert(failure.title, failure.message);
    } else {
      Alert.alert(t('app.couldNotOpenLink'), t('app.openInBrowserInstead', { url }));
    }
  }
}
