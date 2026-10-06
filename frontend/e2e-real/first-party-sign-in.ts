// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: API-level sign-in for the real-backend specs: hosted login page, authorization code, PKCE
// ABOUTME: The Playwright twin of scripts/auth/first-party-sign-in.sh; returns the /oauth/token response

import { createHash, randomBytes } from 'node:crypto';
import type { APIRequestContext, APIResponse } from '@playwright/test';

// The password grant is gone from /oauth/token (carnet#787, RFC 9700 §2.4), so
// a spec signs in the way the apps do:
//
//   1. POST /oauth2/login      the hosted form: credentials + the authorization
//                              request → 302 to /oauth2/authorize + session cookie
//   2. GET  /oauth2/authorize  with that cookie → 302 to redirect_uri?code=…&state=…
//                              (first-party clients skip the consent screen)
//   3. POST /oauth/token       grant_type=authorization_code + the PKCE verifier
//
// The request context's own cookie jar carries the session cookie from step 1
// to step 2, as it carried the auth cookies the password grant set.

/** The mobile app's client: its callback is accepted by every deployment, so a spec never depends on an origin. */
const MOBILE_CLIENT_ID = 'dravr-mobile';
const MOBILE_REDIRECT_URI = 'dravr://auth/callback';

/** The JSON `/oauth/token` answers a sign-in with. */
export interface FirstPartyTokenResponse {
  access_token: string;
  token_type: string;
  expires_in: number;
  refresh_token?: string;
  csrf_token?: string;
  user?: {
    user_id?: string;
    email?: string;
    user_status?: string;
    role?: string;
    is_admin?: boolean;
    tenant_id?: string;
    [key: string]: unknown;
  };
  [key: string]: unknown;
}

/** Where a sign-in stopped: the step that refused it and the response it refused with. */
export type SignInOutcome =
  | { ok: true; tokens: FirstPartyTokenResponse }
  | { ok: false; step: 'login' | 'authorize' | 'token'; status: number; reason: string };

function base64Url(bytes: Buffer): string {
  return bytes.toString('base64').replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

function refused(step: 'login' | 'authorize' | 'token', response: APIResponse, reason: string): SignInOutcome {
  return { ok: false, step, status: response.status(), reason };
}

/**
 * Run the first-party sign-in for `email` as the mobile app and report how it
 * ended, without asserting — for specs that expect a refusal. `scope:
 * 'offline_access'` also returns a refresh token.
 */
export async function attemptSignIn(
  ctx: APIRequestContext,
  email: string,
  password: string,
  options: { scope?: string } = {},
): Promise<SignInOutcome> {
  const codeVerifier = base64Url(randomBytes(32));
  const codeChallenge = base64Url(createHash('sha256').update(codeVerifier).digest());
  const state = base64Url(randomBytes(16));

  const login = await ctx.post('/oauth2/login', {
    form: {
      client_id: MOBILE_CLIENT_ID,
      redirect_uri: MOBILE_REDIRECT_URI,
      response_type: 'code',
      state,
      scope: '',
      code_challenge: codeChallenge,
      code_challenge_method: 'S256',
      resource: '',
      email,
      password,
    },
    maxRedirects: 0,
  });
  const authorizeUrl = login.headers()['location'];
  if (login.status() === 429) {
    return refused('login', login, 'too many refused passwords');
  }
  if (login.status() < 300 || login.status() >= 400 || !authorizeUrl) {
    return refused('login', login, 'the hosted login page refused the credentials');
  }

  const authorize = await ctx.get(authorizeUrl, { maxRedirects: 0 });
  const callback = authorize.headers()['location'] ?? '';
  if (!callback.startsWith(`${MOBILE_REDIRECT_URI}?`)) {
    return refused('authorize', authorize, `no redirect to ${MOBILE_REDIRECT_URI}`);
  }
  const params = new URLSearchParams(callback.slice(MOBILE_REDIRECT_URI.length + 1));
  const code = params.get('code');
  if (params.get('error') || params.get('state') !== state || !code) {
    return refused('authorize', authorize, params.get('error') ?? 'no code, or a mismatched state');
  }

  const form: Record<string, string> = {
    grant_type: 'authorization_code',
    client_id: MOBILE_CLIENT_ID,
    code,
    redirect_uri: MOBILE_REDIRECT_URI,
    code_verifier: codeVerifier,
  };
  if (options.scope) {
    form.scope = options.scope;
  }
  const token = await ctx.post('/oauth/token', { form });
  const body = (await token.json().catch(() => ({}))) as FirstPartyTokenResponse & { error?: string };
  if (!token.ok() || typeof body.access_token !== 'string' || body.access_token.length === 0) {
    return refused('token', token, body.error ?? 'no access_token');
  }
  return { ok: true, tokens: body };
}

/** Sign `email` in and return the token response; fails the spec, naming the refusing step, otherwise. */
export async function signIn(
  ctx: APIRequestContext,
  email: string,
  password: string,
  options: { scope?: string } = {},
): Promise<FirstPartyTokenResponse> {
  const outcome = await attemptSignIn(ctx, email, password, options);
  if (!outcome.ok) {
    throw new Error(
      `sign-in of ${email} refused at ${outcome.step} (${outcome.status}): ${outcome.reason} — ` +
        're-run the setup script, or source .envrc for ADMIN_EMAIL/ADMIN_PASSWORD',
    );
  }
  return outcome.tokens;
}

/** Sign `email` in and return just the bearer. */
export async function accessToken(ctx: APIRequestContext, email: string, password: string): Promise<string> {
  return (await signIn(ctx, email, password)).access_token;
}
