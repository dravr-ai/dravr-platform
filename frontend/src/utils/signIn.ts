// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The web app's half of the hosted sign-in (carnet#787): WebCrypto PKCE, the pending sign-in kept across the redirect
// ABOUTME: The password is typed on the server's page; the app only starts the round trip and redeems the code it returns with

import type { PkceCrypto } from '@pierre/api-client';

/** Where the hosted sign-in sends the athlete back. Registered server-side for `dravr-web`. */
export const SIGN_IN_CALLBACK_PATH = '/auth/callback';

/** The sessionStorage key the pending sign-in survives the redirect under. */
export const PENDING_SIGN_IN_KEY = 'pierre_pending_sign_in';

/** What the app keeps between opening the hosted sign-in and its return. */
export interface PendingSignIn {
  codeVerifier: string;
  state: string;
  redirectUri: string;
  /** The followed link the sign-in interrupted, restored once the session exists. */
  deepLink: string | null;
}

/** PKCE's two primitives, from the browser's WebCrypto. */
export const webPkceCrypto: PkceCrypto = {
  randomBytes(length: number): Uint8Array {
    return crypto.getRandomValues(new Uint8Array(length));
  },
  async sha256(input: string): Promise<Uint8Array> {
    const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(input));
    return new Uint8Array(digest);
  },
};

/** The redirect URI this origin signs in with. */
export function signInRedirectUri(): string {
  return `${window.location.origin}${SIGN_IN_CALLBACK_PATH}`;
}

/** Whether this page load is the hosted sign-in's return leg. */
export function isSignInCallbackPath(): boolean {
  return window.location.pathname === SIGN_IN_CALLBACK_PATH;
}

/**
 * Keep the sign-in for the return leg. sessionStorage is per tab, so a
 * callback opened in another tab finds nothing and is refused.
 */
export function storePendingSignIn(pending: PendingSignIn): void {
  sessionStorage.setItem(PENDING_SIGN_IN_KEY, JSON.stringify(pending));
}

function isPendingSignIn(value: unknown): value is PendingSignIn {
  if (typeof value !== 'object' || value === null) return false;
  const v = value as Record<string, unknown>;
  return (
    typeof v.codeVerifier === 'string' &&
    typeof v.state === 'string' &&
    typeof v.redirectUri === 'string' &&
    (v.deepLink === null || typeof v.deepLink === 'string')
  );
}

/**
 * The pending sign-in, removed as it is read: a verifier redeems one code,
 * so it is never offered twice.
 */
export function takePendingSignIn(): PendingSignIn | null {
  const raw = sessionStorage.getItem(PENDING_SIGN_IN_KEY);
  sessionStorage.removeItem(PENDING_SIGN_IN_KEY);
  if (!raw) return null;
  try {
    const parsed: unknown = JSON.parse(raw);
    return isPendingSignIn(parsed) ? parsed : null;
  } catch {
    return null;
  }
}
