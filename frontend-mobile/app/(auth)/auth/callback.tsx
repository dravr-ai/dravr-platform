// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The hosted sign-in's return path (dravr://auth/callback) as a route, so its deep link is never "unmatched"
// ABOUTME: The code is redeemed by the auth session that opened the browser; this screen only steps back out of the way

import { useEffect } from 'react';
import { useRouter } from 'expo-router';

/**
 * Where the hosted sign-in's redirect lands when it also arrives as a deep link.
 *
 * `WebBrowser.openAuthSessionAsync` reads the redirect itself and hands it to
 * the sign-in that opened it (`src/utils/hostedSignIn.ts`). On Android, and in
 * Expo Go, the same URL also reaches Expo Router as an ordinary deep link,
 * which without this file renders the "Unmatched Route" screen over the login.
 * The route sits in the `(auth)` group so the root layout's guard treats it as
 * part of sign-in: it returns to the login screen it covered, and once the
 * session exists the guard moves on to Home as it does after any sign-in.
 * It reads nothing from the URL — a code is only redeemed with the verifier
 * the opening call holds.
 */
export default function SignInCallbackRoute() {
  const router = useRouter();

  useEffect(() => {
    if (router.canGoBack()) {
      router.back();
    } else {
      router.replace('/(auth)/login');
    }
  }, [router]);

  return null;
}
