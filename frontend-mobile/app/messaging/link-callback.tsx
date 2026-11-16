// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A Slack/Discord link's return path (dravr://messaging/link-callback) as a route, so its deep link is never "unmatched"
// ABOUTME: The link is finished by the auth session that opened the browser; this screen only steps back out of the way

import { useEffect } from 'react';
import { useRouter } from 'expo-router';
import { HOME_ROUTE } from '../../src/navigation/routes';

/**
 * Where a Slack or Discord link's redirect lands when it also arrives as a deep link.
 *
 * `WebBrowser.openAuthSessionAsync` reads the redirect itself and hands it to
 * the link that opened it (`src/utils/oauthChannelLink.ts`), which finishes the
 * link with this app's login. On Android, and in Expo Go, the same URL also
 * reaches Expo Router as an ordinary deep link, which without this file renders
 * the "Unmatched Route" screen over the screen that started the link. It sits
 * outside the route groups, so the root layout's guard still decides where a
 * signed-out or onboarding athlete belongs. It reads nothing from the URL: only
 * the opening call finishes a link.
 */
export default function ChannelLinkCallbackRoute() {
  const router = useRouter();

  useEffect(() => {
    if (router.canGoBack()) {
      router.back();
    } else {
      router.replace(HOME_ROUTE);
    }
  }, [router]);

  return null;
}
