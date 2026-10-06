// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the phone's hosted sign-in round trip: PKCE request, auth session, callback checks, code redemption
// ABOUTME: Runs the real api-client's beginSignIn; only the browser sheet and the token request are scripted

import { createHash } from 'crypto';
import * as WebBrowser from 'expo-web-browser';
import { authApi } from '../../services/api';
import type { LoginResponse } from '../../types';
import { SignInRefusedError, signInWithHostedPage } from '../hostedSignIn';

const REDIRECT_URI = 'exp://127.0.0.1:8082/--/auth/callback';

jest.mock('expo-linking', () => ({
  createURL: jest.fn((path: string) => `exp://127.0.0.1:8082/--/${path}`),
}));

const openAuthSession = WebBrowser.openAuthSessionAsync as jest.Mock;

const SESSION: LoginResponse = {
  access_token: 'jwt-token',
  token_type: 'Bearer',
  refresh_token: 'refresh-token',
  csrf_token: 'csrf-token',
  user: {
    id: 'u-1',
    user_id: 'u-1',
    email: 'athlete@example.com',
    is_admin: false,
    role: 'user',
    user_status: 'active',
    tier: 'starter',
    created_at: '2026-10-06T00:00:00Z',
  },
};

/** The authorization request the sheet was opened on, as the server reads it. */
function openedRequest(): URLSearchParams {
  const [url] = openAuthSession.mock.calls[0] as [string];
  return new URL(url).searchParams;
}

/** The sheet closes on a redirect to the callback carrying `query`, built from the opened request. */
function redirectWith(query: (request: URLSearchParams) => Record<string, string>): void {
  openAuthSession.mockImplementationOnce(async (url: string) => {
    const request = new URL(url).searchParams;
    return { type: 'success', url: `${REDIRECT_URI}?${new URLSearchParams(query(request)).toString()}` };
  });
}

describe('signInWithHostedPage', () => {
  let completeSignIn: jest.SpyInstance;

  beforeEach(() => {
    jest.clearAllMocks();
    completeSignIn = jest.spyOn(authApi, 'completeSignIn').mockResolvedValue(SESSION);
  });

  afterEach(() => {
    completeSignIn.mockRestore();
  });

  it('opens the hosted sign-in as dravr-mobile with an S256 challenge, in an ephemeral session', async () => {
    redirectWith((request) => ({ code: 'the-code', state: request.get('state') ?? '' }));

    await signInWithHostedPage();

    const [url, returnUrl, options] = openAuthSession.mock.calls[0] as [string, string, object];
    expect(new URL(url).pathname).toBe('/oauth2/authorize');
    expect(returnUrl).toBe(REDIRECT_URI);
    // No shared Safari cookies: a sign-in after a sign-out shows the page again.
    expect(options).toEqual({ preferEphemeralSession: true });

    const request = openedRequest();
    expect(request.get('response_type')).toBe('code');
    expect(request.get('client_id')).toBe('dravr-mobile');
    expect(request.get('redirect_uri')).toBe(REDIRECT_URI);
    expect(request.get('code_challenge_method')).toBe('S256');
    expect(request.get('state')).toMatch(/^[A-Za-z0-9_-]{22}$/);

    // The verifier redeemed is the one the challenge was computed from.
    const { codeVerifier } = completeSignIn.mock.calls[0][0] as { codeVerifier: string };
    expect(codeVerifier).toMatch(/^[A-Za-z0-9_-]{43}$/);
    const challenge = createHash('sha256').update(codeVerifier).digest('base64url');
    expect(request.get('code_challenge')).toBe(challenge);
  });

  it('redeems the returned code with its verifier and redirect URI', async () => {
    redirectWith((request) => ({ code: 'the-code', state: request.get('state') ?? '' }));

    await expect(signInWithHostedPage()).resolves.toBe(SESSION);

    expect(completeSignIn).toHaveBeenCalledTimes(1);
    expect(completeSignIn).toHaveBeenCalledWith({
      code: 'the-code',
      codeVerifier: expect.any(String),
      redirectUri: REDIRECT_URI,
    });
  });

  it.each(['cancel', 'dismiss', 'locked'])('resolves null and redeems nothing when the session ends with %s', async (type) => {
    openAuthSession.mockResolvedValueOnce({ type });

    await expect(signInWithHostedPage()).resolves.toBeNull();
    expect(completeSignIn).not.toHaveBeenCalled();
  });

  it('refuses a callback whose state is not this sign-in’s', async () => {
    redirectWith(() => ({ code: 'forged-code', state: 'someone-elses-state' }));

    const failure = signInWithHostedPage();

    await expect(failure).rejects.toBeInstanceOf(SignInRefusedError);
    await expect(failure).rejects.toMatchObject({ code: 'state_mismatch' });
    expect(completeSignIn).not.toHaveBeenCalled();
  });

  it('refuses a redirect to anywhere but the callback it asked for', async () => {
    openAuthSession.mockImplementationOnce(async (url: string) => {
      const state = new URL(url).searchParams.get('state') ?? '';
      return { type: 'success', url: `dravr://elsewhere?code=the-code&state=${state}` };
    });

    await expect(signInWithHostedPage()).rejects.toMatchObject({ code: 'state_mismatch' });
    expect(completeSignIn).not.toHaveBeenCalled();
  });

  it('surfaces the error the server sent back, without redeeming anything', async () => {
    redirectWith((request) => ({
      error: 'access_denied',
      error_description: 'Account suspended',
      state: request.get('state') ?? '',
    }));

    await expect(signInWithHostedPage()).rejects.toMatchObject({
      name: 'SignInRefusedError',
      code: 'access_denied',
      description: 'Account suspended',
    });
    expect(completeSignIn).not.toHaveBeenCalled();
  });

  it('refuses a callback that carries no code', async () => {
    redirectWith((request) => ({ state: request.get('state') ?? '' }));

    await expect(signInWithHostedPage()).rejects.toMatchObject({ code: 'invalid_request' });
    expect(completeSignIn).not.toHaveBeenCalled();
  });

  it('lets a failed code redemption reach the caller as the API error', async () => {
    const refused = Object.assign(new Error('Request failed with status code 400'), {
      response: { status: 400, data: { error: 'invalid_grant' } },
    });
    completeSignIn.mockRejectedValueOnce(refused);
    redirectWith((request) => ({ code: 'stale-code', state: request.get('state') ?? '' }));

    await expect(signInWithHostedPage()).rejects.toBe(refused);
  });
});
