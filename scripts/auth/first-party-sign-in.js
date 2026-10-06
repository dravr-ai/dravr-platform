// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Signs a user in headlessly as Dravr's own app: hosted login page, authorization code, PKCE
// ABOUTME: The Node twin of first-party-sign-in.sh, shared by the SDK type generator and the SDK test helpers

'use strict';

// The password grant is gone from /oauth/token (carnet#787, RFC 9700 §2.4), so
// a script signs in the way the apps do:
//
//   1. POST /oauth2/login      the hosted form: credentials + the authorization
//                              request → 302 to /oauth2/authorize + session cookie
//   2. GET  /oauth2/authorize  with that cookie → 302 to redirect_uri?code=…&state=…
//                              (first-party clients skip the consent screen)
//   3. POST /oauth/token       grant_type=authorization_code + the PKCE verifier
//
// CommonJS on purpose: scripts/sdk/generate-sdk-types.js and sdk/test/helpers
// are both CommonJS. Needs a global fetch whose redirect: 'manual' exposes the
// real 3xx response (Node 18+, Bun).

const crypto = require('node:crypto');

const MOBILE_CLIENT_ID = 'dravr-mobile';
const WEB_CLIENT_ID = 'dravr-web';
const MOBILE_REDIRECT_URI = 'dravr://auth/callback';
const CALLBACK_PATH = '/auth/callback';

/** Thrown when the server refuses the sign-in; `status` is the HTTP status of the refusing step. */
class FirstPartySignInError extends Error {
  constructor(message, status) {
    super(message);
    this.name = 'FirstPartySignInError';
    this.status = status;
  }
}

function base64Url(buffer) {
  return buffer.toString('base64').replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

/** The redirect URI `clientId` receives its code at on a server reached at `baseUrl`. */
function redirectUriFor(clientId, baseUrl, frontendUrl) {
  if (clientId === MOBILE_CLIENT_ID) return MOBILE_REDIRECT_URI;
  if (clientId === WEB_CLIENT_ID) {
    const origin = (frontendUrl || new URL(baseUrl).origin).replace(/\/+$/, '');
    return `${origin}${CALLBACK_PATH}`;
  }
  throw new FirstPartySignInError(`client_id must be ${MOBILE_CLIENT_ID} or ${WEB_CLIENT_ID}, got '${clientId}'`);
}

/** `name=value` pairs of every Set-Cookie on `response`, for the next request's Cookie header. */
function cookiesOf(response) {
  const headers = response.headers;
  const all =
    typeof headers.getSetCookie === 'function'
      ? headers.getSetCookie()
      : (headers.get('set-cookie') || '').split(/,(?=\s*[^;,=\s]+=)/);
  return all
    .map((cookie) => cookie.split(';')[0].trim())
    .filter((pair) => pair.includes('='))
    .join('; ');
}

/**
 * Sign `email` in as Dravr's own app and return the `/oauth/token` JSON
 * (`access_token`, `token_type`, `expires_in`, `user`, `csrf_token`,
 * `refresh_token` when `scope` is `offline_access`).
 *
 * @param {object} options
 * @param {string} options.baseUrl    the API origin, e.g. http://localhost:8081
 * @param {string} options.email
 * @param {string} options.password
 * @param {string} [options.clientId] dravr-mobile (default; dravr://auth/callback,
 *   accepted by every deployment) or dravr-web (`<frontendUrl or baseUrl origin>/auth/callback`)
 * @param {string} [options.frontendUrl] the web app's origin, for dravr-web; must
 *   equal the server's FRONTEND_URL or issuer origin
 * @param {string} [options.scope]    sent on the token request; offline_access adds a refresh_token
 * @returns {Promise<object>} the token response
 * @throws {FirstPartySignInError} on a refused password (status 200/4xx), rate limit (429), or missing code
 */
async function firstPartySignIn({ baseUrl, email, password, clientId = MOBILE_CLIENT_ID, frontendUrl, scope }) {
  const base = baseUrl.replace(/\/+$/, '');
  const redirectUri = redirectUriFor(clientId, base, frontendUrl);
  const codeVerifier = base64Url(crypto.randomBytes(32));
  const codeChallenge = base64Url(crypto.createHash('sha256').update(codeVerifier).digest());
  const state = base64Url(crypto.randomBytes(16));

  // Step 1: the hosted login form.
  const login = await fetch(`${base}/oauth2/login`, {
    method: 'POST',
    redirect: 'manual',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams({
      client_id: clientId,
      redirect_uri: redirectUri,
      response_type: 'code',
      state,
      scope: '',
      code_challenge: codeChallenge,
      code_challenge_method: 'S256',
      resource: '',
      email,
      password,
    }).toString(),
  });
  const authorizeLocation = login.headers.get('location');
  if (login.status === 429) {
    throw new FirstPartySignInError(`sign-in for ${email} refused: too many refused passwords`, 429);
  }
  if (login.status < 300 || login.status >= 400 || !authorizeLocation) {
    throw new FirstPartySignInError(
      `sign-in for ${email} refused by the hosted login page (HTTP ${login.status}): wrong password, unknown or suspended account`,
      login.status
    );
  }

  // Step 2: the authorization request, signed in by the session cookie.
  const authorize = await fetch(new URL(authorizeLocation, `${base}/`).toString(), {
    redirect: 'manual',
    headers: { Cookie: cookiesOf(login) },
  });
  const callback = authorize.headers.get('location') || '';
  if (!callback.startsWith(`${redirectUri}?`)) {
    throw new FirstPartySignInError(
      `/oauth2/authorize answered HTTP ${authorize.status} without redirecting to ${redirectUri}` +
        ' (redirect_uri not allowed for this client on this server? check FRONTEND_URL)',
      authorize.status
    );
  }
  const params = new URLSearchParams(callback.slice(redirectUri.length + 1));
  if (params.get('error')) {
    throw new FirstPartySignInError(
      `authorization refused: ${params.get('error')} ${params.get('error_description') || ''}`.trim(),
      authorize.status
    );
  }
  if (params.get('state') !== state) {
    throw new FirstPartySignInError('authorization returned a mismatched state', authorize.status);
  }
  const code = params.get('code');
  if (!code) throw new FirstPartySignInError('authorization redirect carried no code', authorize.status);

  // Step 3: redeem the code with the PKCE verifier.
  const tokenForm = new URLSearchParams({
    grant_type: 'authorization_code',
    client_id: clientId,
    code,
    redirect_uri: redirectUri,
    code_verifier: codeVerifier,
  });
  if (scope) tokenForm.set('scope', scope);
  const token = await fetch(`${base}/oauth/token`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    body: tokenForm.toString(),
  });
  const body = await token.json().catch(() => ({}));
  if (!token.ok || typeof body.access_token !== 'string' || !body.access_token) {
    const reason = [body.error, body.error_description].filter(Boolean).join(': ');
    throw new FirstPartySignInError(`token exchange failed (HTTP ${token.status})${reason ? `: ${reason}` : ''}`, token.status);
  }
  return body;
}

module.exports = {
  firstPartySignIn,
  FirstPartySignInError,
  MOBILE_CLIENT_ID,
  WEB_CLIENT_ID,
};
