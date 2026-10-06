// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The phone's sign-in: the server's hosted login page in the system browser, authorization code + PKCE
// ABOUTME: The password is typed on the server's page, never into the app (carnet#787, RFC 8252)

import * as Crypto from 'expo-crypto';
import * as Linking from 'expo-linking';
import * as WebBrowser from 'expo-web-browser';
import { readSignInCallback, type PkceCrypto } from '@pierre/api-client';
import type { LoginResponse } from '../types';
import { authApi } from '../services/api';
import { i18n } from '@pierre/i18n';

/**
 * The path the hosted sign-in returns to.
 *
 * `Linking.createURL` turns it into `dravr://auth/callback` in a build and
 * `exp://<host>/--/auth/callback` in Expo Go. The server sends a
 * `dravr-mobile` code to the first only, and to the second only when it runs
 * with `OAUTH_ALLOW_EXPO_GO_REDIRECT=true` (local development and CI). The
 * route `app/(auth)/auth/callback.tsx` answers the same path, for the
 * platforms that also deliver the redirect to the app as a deep link.
 */
export const SIGN_IN_CALLBACK_PATH = 'auth/callback';

/** PKCE's randomness and SHA-256 from expo-crypto: native on both platforms, no WebCrypto in Hermes. */
export const expoPkceCrypto: PkceCrypto = {
  randomBytes: (length) => Crypto.getRandomBytes(length),
  async sha256(input) {
    // The verifier is base64url, so every character is one ASCII byte.
    const bytes = Uint8Array.from(input, (ch) => ch.charCodeAt(0));
    const digest = await Crypto.digest(Crypto.CryptoDigestAlgorithm.SHA256, bytes);
    return new Uint8Array(digest);
  },
};

/**
 * The hosted sign-in came back without a code: the server refused it
 * (`error` on the callback), or the callback was not this sign-in's
 * (`state_mismatch`) or carried nothing to redeem (`invalid_request`).
 */
export class SignInRefusedError extends Error {
  readonly code: string;
  readonly description?: string;

  constructor(code: string, description?: string) {
    super(description ? `${code}: ${description}` : code);
    this.name = 'SignInRefusedError';
    this.code = code;
    this.description = description;
  }
}

/** The query string of a callback URL, without its fragment. */
function callbackQuery(url: string): string {
  const withoutFragment = url.split('#')[0];
  const queryStart = withoutFragment.indexOf('?');
  return queryStart === -1 ? '' : withoutFragment.slice(queryStart + 1);
}

/**
 * Sign in through the server's hosted login page.
 *
 * Opens `/oauth2/authorize` in an auth session (ASWebAuthenticationSession on
 * iOS, a Custom Tab on Android), waits for the redirect back to the app, and
 * redeems the code with the PKCE verifier only this call holds. Resolves to
 * the session — tokens and user already stored by the API client — or `null`
 * when the athlete closed the browser. A refused or foreign callback throws
 * {@link SignInRefusedError}; a failed code redemption throws the API error.
 *
 * The iOS session is ephemeral: it shares no cookies with Safari, so a sign-in
 * after a sign-out shows the login page again instead of silently reusing the
 * authorization server's session for the account that just left, and iOS does
 * not ask the athlete to let the app "use" the server's domain to sign in.
 */
export async function signInWithHostedPage(): Promise<LoginResponse | null> {
  const redirectUri = Linking.createURL(SIGN_IN_CALLBACK_PATH);
  const request = await authApi.beginSignIn(redirectUri, expoPkceCrypto, i18n.language);

  const result = await WebBrowser.openAuthSessionAsync(request.authorizeUrl, request.redirectUri, {
    preferEphemeralSession: true,
  });
  if (result.type !== 'success') {
    // cancel / dismiss / locked: the athlete closed the sheet, or another
    // auth session already holds the browser. Nothing to report.
    return null;
  }

  if (!result.url.startsWith(request.redirectUri)) {
    throw new SignInRefusedError('state_mismatch');
  }
  const callback = readSignInCallback(new URLSearchParams(callbackQuery(result.url)), request.state);
  if (callback.kind === 'error') {
    throw new SignInRefusedError(callback.error, callback.description);
  }

  return authApi.completeSignIn({
    code: callback.code,
    codeVerifier: request.codeVerifier,
    redirectUri: request.redirectUri,
  });
}
