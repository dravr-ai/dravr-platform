// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Lifecycle tests for the OAuth authorization wait, its teardown, and token refresh
// ABOUTME: Proves the wait is bounded, the listener never outlives a flow, and refresh is single-flight

const {
  closeServer,
  freePort,
  httpRequest,
  makeProvider,
  portIsFree,
  startPierreStub,
  startProvider,
  stopProvider,
} = require('./oauth-callback-harness.js');

describe('authorization wait lifecycle', () => {
  test('an authorization nobody completes times out and takes the whole flow down with it', async () => {
    const { provider, port } = await startProvider({ authorizationTimeoutMs: 200 });
    await provider.saveCodeVerifier('verifier-that-must-not-survive');
    const state = await provider.state();
    expect(state).toHaveLength(32);
    await provider.startCallbackServer();

    await expect(provider.completeAuthorization()).rejects.toThrow(
      /authorization was not completed within/i,
    );

    expect(provider.codeVerifierValue).toBeUndefined();
    expect(provider.stateValue).toBeUndefined();
    expect(provider.callbackAuthToken).toBeUndefined();
    expect(provider.callbackServer).toBeUndefined();
    expect(provider.authorizationPending).toBeUndefined();

    // Nothing is listening on the callback port any more.
    await expect(portIsFree(port)).resolves.toBe(port);
  });

  test('the timeout error tells the user how to recover', async () => {
    const { provider } = await startProvider({ authorizationTimeoutMs: 100 });
    await provider.startCallbackServer();

    const error = await provider.completeAuthorization().then(
      () => null,
      (e) => e,
    );

    expect(error.code).toBe('TIMEOUT_ERROR');
    expect(error.message).toMatch(/run the connect tool again/i);
  });

  test('a completed authorization exchanges the code and leaves nothing behind', async () => {
    const pierre = await startPierreStub({
      '/oauth2/token': () => ({
        body: {
          access_token: 'issued-jwt',
          token_type: 'Bearer',
          expires_in: 3600,
          refresh_token: 'issued-refresh',
          scope: 'read:fitness write:fitness',
        },
      }),
    });

    const { provider, port } = await startProvider({}, pierre.url);
    provider.clientInfo = { client_id: 'client-abc', client_secret: 'secret-xyz' };
    await provider.saveCodeVerifier('verifier-that-must-not-survive');
    const state = await provider.state();
    await provider.startCallbackServer();

    try {
      const completing = provider.completeAuthorization();
      const callbackResponse = await httpRequest(port, {
        method: 'GET',
        path: `/oauth/callback?code=authorization-code-1&state=${state}`,
      });
      expect(callbackResponse.status).toBe(200);
      await completing;

      const tokens = await provider.tokens();
      expect(tokens.access_token).toBe('issued-jwt');
      expect(tokens.refresh_token).toBe('issued-refresh');

      // The exchange used the verifier and named the port that was actually bound.
      const exchange = pierre.requests.find((r) => r.url === '/oauth2/token');
      const form = new URLSearchParams(exchange.body);
      expect(form.get('code')).toBe('authorization-code-1');
      expect(form.get('code_verifier')).toBe('verifier-that-must-not-survive');
      expect(form.get('redirect_uri')).toBe(`http://localhost:${port}/oauth/callback`);

      // Success tears down exactly like failure does.
      expect(provider.codeVerifierValue).toBeUndefined();
      expect(provider.stateValue).toBeUndefined();
      expect(provider.callbackServer).toBeUndefined();
      await expect(portIsFree(port)).resolves.toBe(port);
    } finally {
      stopProvider(provider);
      await closeServer(pierre.server);
    }
  });

  test('a callback whose state does not match is refused and still tears the flow down', async () => {
    const pierre = await startPierreStub({});
    const { provider, port } = await startProvider({}, pierre.url);
    await provider.saveCodeVerifier('verifier-that-must-not-survive');
    await provider.state();
    await provider.startCallbackServer();

    try {
      // The rejection lands while the callback request is still in flight, so the
      // outcome is captured up front rather than awaited after the fact.
      const outcome = provider.completeAuthorization().then(
        () => null,
        (e) => e,
      );
      await httpRequest(port, {
        method: 'GET',
        path: '/oauth/callback?code=authorization-code-1&state=forged-state',
      });

      const error = await outcome;
      expect(error.message).toMatch(/state parameter mismatch/i);
      expect(provider.codeVerifierValue).toBeUndefined();
      expect(provider.callbackServer).toBeUndefined();
      await expect(portIsFree(port)).resolves.toBe(port);
      expect(pierre.requests.filter((r) => r.url === '/oauth2/token')).toHaveLength(0);
    } finally {
      stopProvider(provider);
      await closeServer(pierre.server);
    }
  });
});

describe('token refresh single-flight', () => {
  const storedSession = () => ({
    pierre: {
      access_token: 'stale-access',
      refresh_token: 'rotating-refresh',
      token_type: 'Bearer',
      expires_in: 3600,
      scope: 'read:fitness write:fitness',
      saved_at: Math.floor(Date.now() / 1000),
    },
  });

  test('concurrent callers share one refresh and keep the renewed session', async () => {
    // The server rotates the refresh token: the first attempt renews, any concurrent
    // second attempt presents a retired token and is told the session is invalid.
    const pierre = await startPierreStub({
      '/oauth2/validate-and-refresh': (callNumber) =>
        callNumber === 1
          ? {
              body: {
                status: 'refreshed',
                access_token: 'renewed-access',
                refresh_token: 'renewed-refresh',
                token_type: 'Bearer',
                expires_in: 3600,
              },
            }
          : {
              body: {
                status: 'invalid',
                reason: 'refresh token already used',
                requires_full_reauth: true,
              },
            },
    });

    const provider = makeProvider({}, pierre.url);
    provider.allStoredTokens = storedSession();

    try {
      const [first, second, third] = await Promise.all([
        provider.tokens(),
        provider.tokens(),
        provider.tokens(),
      ]);

      expect(
        pierre.requests.filter((r) => r.url === '/oauth2/validate-and-refresh'),
      ).toHaveLength(1);
      expect(first.access_token).toBe('renewed-access');
      expect(second).toBe(first);
      expect(third).toBe(first);

      // The session the winner renewed is still there, in memory and in storage.
      expect(provider.allStoredTokens.pierre.access_token).toBe('renewed-access');
      expect(provider.allStoredTokens.pierre.refresh_token).toBe('renewed-refresh');
      expect(provider.getTokenStatus().pierre).toBe(true);
    } finally {
      await closeServer(pierre.server);
    }
  });

  test('single-flight is per attempt, not a latch that suppresses later refreshes', async () => {
    const pierre = await startPierreStub({
      '/oauth2/validate-and-refresh': (callNumber) => ({
        body: {
          status: 'refreshed',
          access_token: `renewed-access-${callNumber}`,
          refresh_token: `renewed-refresh-${callNumber}`,
          token_type: 'Bearer',
          expires_in: 3600,
        },
      }),
    });

    const provider = makeProvider({}, pierre.url);
    provider.allStoredTokens = storedSession();

    try {
      const first = await provider.tokens();
      expect(first.access_token).toBe('renewed-access-1');

      provider.savedTokens = undefined;
      const second = await provider.tokens();
      expect(second.access_token).toBe('renewed-access-2');
      expect(
        pierre.requests.filter((r) => r.url === '/oauth2/validate-and-refresh'),
      ).toHaveLength(2);
    } finally {
      await closeServer(pierre.server);
    }
  });

  test('a validation request that never completes leaves the stored session untouched', async () => {
    const pierre = await startPierreStub({
      '/oauth2/validate-and-refresh': () => ({ status: 503, body: { error: 'unavailable' } }),
    });

    const provider = makeProvider({}, pierre.url);
    provider.allStoredTokens = storedSession();

    try {
      const tokens = await provider.tokens();

      expect(tokens).toBeUndefined();
      expect(provider.allStoredTokens.pierre.access_token).toBe('stale-access');
      expect(provider.allStoredTokens.pierre.refresh_token).toBe('rotating-refresh');
    } finally {
      await closeServer(pierre.server);
    }
  });

  test('a session the server calls invalid is cleared from memory and storage together', async () => {
    const pierre = await startPierreStub({
      '/oauth2/validate-and-refresh': () => ({
        body: { status: 'invalid', reason: 'refresh token revoked', requires_full_reauth: true },
      }),
    });

    const provider = makeProvider({}, pierre.url);
    provider.allStoredTokens = storedSession();

    try {
      const tokens = await provider.tokens();

      expect(tokens).toBeUndefined();
      expect(provider.allStoredTokens.pierre).toBeUndefined();
      expect(provider.getTokenStatus().pierre).toBe(false);
    } finally {
      await closeServer(pierre.server);
    }
  });
});

describe('callback port lifecycle', () => {
  test('a fresh flow rebinds the port a previous flow released', async () => {
    const port = await freePort();
    const first = await startProvider({ callbackPort: port });
    expect(first.boundPort).toBe(port);
    stopProvider(first.provider);
    await expect(portIsFree(port)).resolves.toBe(port);

    const second = await startProvider({ callbackPort: port });
    try {
      expect(second.boundPort).toBe(port);
      expect(second.provider.redirectUrl).toBe(`http://localhost:${port}/oauth/callback`);
    } finally {
      stopProvider(second.provider);
    }
  });
});
