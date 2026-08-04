// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Security tests for the local OAuth callback listener (token injection, port binding)
// ABOUTME: Proves provider token POSTs must prove flow membership and the published port is held

const net = require('net');
const {
  closeServer,
  freePort,
  httpRequest,
  makeProvider,
  occupyPort,
  startProvider,
  stopProvider,
} = require('./oauth-callback-harness.js');

// What a hostile local process (or a page in the user's browser) would try to plant.
const INJECTED = {
  access_token: 'attacker-access-token',
  refresh_token: 'attacker-refresh-token',
  expires_in: 3600,
};

describe('provider token callback authentication', () => {
  let provider;
  let port;
  let callbackToken;

  beforeEach(async () => {
    ({ provider, port } = await startProvider());
    callbackToken = provider.callbackAuthToken;
  });

  afterEach(() => {
    stopProvider(provider);
  });

  test('the flow secret exists and is not derivable from the endpoint', () => {
    expect(callbackToken).toMatch(/^[0-9a-f]{64}$/);
  });

  test('a POST with no callback token is refused and stores nothing', async () => {
    const res = await httpRequest(port, {
      method: 'POST',
      path: '/oauth/provider-callback/strava',
      json: INJECTED,
    });

    expect(res.status).toBe(403);
    expect(JSON.parse(res.body)).toEqual({
      success: false,
      message: 'Invalid or missing callback authentication token',
    });
    expect(provider.getProviderToken('strava')).toBeUndefined();
    expect(provider.allStoredTokens.providers).toBeUndefined();
  });

  test('a POST carrying the wrong flow secret is refused and stores nothing', async () => {
    const res = await httpRequest(port, {
      method: 'POST',
      path: '/oauth/provider-callback/strava',
      headers: { 'x-callback-token': 'f'.repeat(64) },
      json: INJECTED,
    });

    expect(res.status).toBe(403);
    expect(JSON.parse(res.body).success).toBe(false);
    expect(provider.getProviderToken('strava')).toBeUndefined();
    expect(provider.allStoredTokens.providers).toBeUndefined();
  });

  test('a token of the wrong length is refused rather than crashing the comparison', async () => {
    const res = await httpRequest(port, {
      method: 'POST',
      path: '/oauth/provider-callback/strava',
      headers: { 'x-callback-token': 'short' },
      json: INJECTED,
    });

    expect(res.status).toBe(403);
    expect(provider.allStoredTokens.providers).toBeUndefined();
  });

  test('a browser page cannot plant tokens even holding the flow secret (Origin)', async () => {
    const res = await httpRequest(port, {
      method: 'POST',
      path: '/oauth/provider-callback/strava',
      headers: { 'x-callback-token': callbackToken, Origin: 'https://attacker.example' },
      json: INJECTED,
    });

    expect(res.status).toBe(403);
    expect(JSON.parse(res.body).message).toMatch(/browser-originated/i);
    expect(provider.allStoredTokens.providers).toBeUndefined();
  });

  test('a browser page cannot plant tokens even holding the flow secret (Referer)', async () => {
    const res = await httpRequest(port, {
      method: 'POST',
      path: '/oauth/provider-callback/strava',
      headers: {
        'x-callback-token': callbackToken,
        Referer: 'https://attacker.example/page',
      },
      json: INJECTED,
    });

    expect(res.status).toBe(403);
    expect(provider.allStoredTokens.providers).toBeUndefined();
  });

  test('extra path segments after the provider name are not a callback', async () => {
    const res = await httpRequest(port, {
      method: 'POST',
      path: '/oauth/provider-callback/strava/extra/segments',
      headers: { 'x-callback-token': callbackToken },
      json: INJECTED,
    });

    expect(res.status).toBe(404);
    expect(provider.allStoredTokens.providers).toBeUndefined();
  });

  test('the callback path accepts POST only', async () => {
    const res = await httpRequest(port, {
      method: 'GET',
      path: `/oauth/provider-callback/strava?callback_token=${callbackToken}`,
    });

    expect(res.status).toBe(405);
    expect(JSON.parse(res.body).message).toMatch(/POST only/);
    expect(provider.allStoredTokens.providers).toBeUndefined();
  });

  test('the real notification is accepted and stores the provider token it carried', async () => {
    const waiting = provider.waitForProviderOAuth('strava', 5000);

    const res = await httpRequest(port, {
      method: 'POST',
      path: '/oauth/provider-callback/strava',
      headers: { 'x-callback-token': callbackToken },
      json: {
        access_token: 'strava-access-token',
        refresh_token: 'strava-refresh-token',
        expires_in: 21600,
        token_type: 'Bearer',
        scope: 'activity:read_all',
      },
    });

    expect(res.status).toBe(200);
    expect(JSON.parse(res.body)).toEqual({
      success: true,
      message: 'strava token stored client-side',
    });

    const stored = provider.getProviderToken('strava');
    expect(stored.access_token).toBe('strava-access-token');
    expect(stored.refresh_token).toBe('strava-refresh-token');
    expect(stored.token_type).toBe('Bearer');
    expect(stored.scope).toBe('activity:read_all');
    expect(stored.expires_at).toBeGreaterThan(Date.now());

    await expect(waiting).resolves.toEqual({ provider: 'strava' });
    expect(provider.getTokenStatus().providers.strava).toBe(true);
  });

  test('the flow secret in the query string is accepted for the same flow', async () => {
    const res = await httpRequest(port, {
      method: 'POST',
      path: `/oauth/provider-callback/fitbit?callback_token=${callbackToken}`,
      json: { access_token: 'fitbit-access-token', expires_in: 28800 },
    });

    expect(res.status).toBe(200);
    expect(provider.getProviderToken('fitbit').access_token).toBe('fitbit-access-token');
  });

  test('an authenticated payload without an access token stores nothing', async () => {
    const res = await httpRequest(port, {
      method: 'POST',
      path: '/oauth/provider-callback/strava',
      headers: { 'x-callback-token': callbackToken },
      json: { refresh_token: 'only-a-refresh-token' },
    });

    expect(res.status).toBe(400);
    expect(JSON.parse(res.body).message).toMatch(/no access_token/);
    expect(provider.allStoredTokens.providers).toBeUndefined();
  });

  test('a secret from a finished flow does not authenticate the next one', async () => {
    const firstSecret = provider.callbackAuthToken;
    stopProvider(provider);

    const restarted = await startProvider();
    provider = restarted.provider;

    expect(provider.callbackAuthToken).not.toBe(firstSecret);

    const res = await httpRequest(restarted.port, {
      method: 'POST',
      path: '/oauth/provider-callback/strava',
      headers: { 'x-callback-token': firstSecret },
      json: INJECTED,
    });

    expect(res.status).toBe(403);
    expect(provider.allStoredTokens.providers).toBeUndefined();
  });
});

describe('callback port binding', () => {
  const savedEnv = {};

  beforeEach(() => {
    savedEnv.PIERRE_DISABLE_BROWSER = process.env.PIERRE_DISABLE_BROWSER;
    savedEnv.CI = process.env.CI;
    savedEnv.GITHUB_ACTIONS = process.env.GITHUB_ACTIONS;
  });

  afterEach(() => {
    for (const [key, value] of Object.entries(savedEnv)) {
      if (value === undefined) {
        delete process.env[key];
      } else {
        process.env[key] = value;
      }
    }
  });

  test('the published redirect URI names the port the listener actually holds', async () => {
    const { provider, port, boundPort } = await startProvider();
    try {
      expect(boundPort).toBe(port);
      expect(provider.callbackServer.address().port).toBe(port);
      expect(provider.redirectUrl).toBe(`http://localhost:${port}/oauth/callback`);
    } finally {
      stopProvider(provider);
    }
  });

  test('an occupied callback port fails hard and never moves to an unpublished port', async () => {
    const port = await freePort();
    const squatter = await occupyPort(port);

    const attemptedPorts = [];
    const originalListen = net.Server.prototype.listen;
    net.Server.prototype.listen = function patched(...args) {
      if (typeof args[0] === 'number') {
        attemptedPorts.push(args[0]);
      }
      return originalListen.apply(this, args);
    };

    const provider = makeProvider({ callbackPort: port });
    try {
      const error = await provider.ensureCallbackServerBound().then(
        () => null,
        (e) => e,
      );

      expect(error).not.toBeNull();
      expect(error.code).toBe('CONFIG_ERROR');
      expect(error.message).toContain(`port ${port} is already in use`);
      expect(error.message).toContain('--callback-port');
      expect(provider.callbackServer).toBeUndefined();

      // Only the published port was ever attempted: a listener on any other port would
      // be one the authorization server was never told about.
      expect([...new Set(attemptedPorts)]).toEqual([port]);
    } finally {
      net.Server.prototype.listen = originalListen;
      stopProvider(provider);
      await closeServer(squatter);
    }
  });

  test('the authorization request is not sent when the callback port cannot be bound', async () => {
    // The refusal must come from the bind failure, not from the non-interactive guard.
    delete process.env.PIERRE_DISABLE_BROWSER;
    delete process.env.CI;
    delete process.env.GITHUB_ACTIONS;

    const port = await freePort();
    const squatter = await occupyPort(port);
    const provider = makeProvider({ callbackPort: port });

    try {
      await expect(
        provider.redirectToAuthorization(
          new URL('http://localhost:8081/oauth2/authorize?client_id=test'),
        ),
      ).rejects.toThrow(new RegExp(`port ${port} is already in use`));
    } finally {
      stopProvider(provider);
      await closeServer(squatter);
    }
  });
});
