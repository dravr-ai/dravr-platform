// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests that a token refresh persists the pair it minted, whoever triggered it
// ABOUTME: Covers the direct validateAndRefreshToken call and the bridge's tool-call retry

const { PierreMcpClient } = require('../../dist/index.js');
const {
  closeServer,
  makeProvider,
  startPierreStub,
} = require('./oauth-callback-harness.js');

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

/** Records what would have gone into the OS keychain, without touching it. */
function recordingStorage(writes) {
  return {
    saveTokens: async (tokens) => {
      writes.push(JSON.parse(JSON.stringify(tokens)));
    },
    loadTokens: async () => ({}),
    clearTokens: async () => {},
  };
}

/** An access token the bridge can decode a user id out of. */
function jwtFor(subject) {
  const payload = Buffer.from(JSON.stringify({ sub: subject })).toString('base64');
  return `header.${payload}.signature`;
}

describe('a refresh persists the pair it minted', () => {
  test('validateAndRefreshToken writes the renewed pair even when its caller ignores the result', async () => {
    const pierre = await startPierreStub({
      '/oauth2/validate-and-refresh': () => ({
        body: {
          status: 'refreshed',
          access_token: 'renewed-access',
          refresh_token: 'renewed-refresh',
          token_type: 'Bearer',
          expires_in: 3600,
        },
      }),
    });

    const provider = makeProvider({}, pierre.url);
    const writes = [];
    provider.secureStorage = recordingStorage(writes);
    provider.allStoredTokens = storedSession();
    provider.savedTokens = { ...storedSession().pierre };

    try {
      const result = await provider.validateAndRefreshToken('stale-access', 'rotating-refresh');
      expect(result.status).toBe('refreshed');

      // The caller did nothing with the response; the session moved on regardless.
      const tokens = await provider.tokens();
      expect(tokens.access_token).toBe('renewed-access');
      expect(tokens.refresh_token).toBe('renewed-refresh');
      expect(tokens.scope).toBe('read:fitness write:fitness');

      // The consumed refresh token is gone from storage, so the next startup validation
      // presents the live one instead of a token the server has already retired.
      expect(provider.allStoredTokens.pierre.access_token).toBe('renewed-access');
      expect(provider.allStoredTokens.pierre.refresh_token).toBe('renewed-refresh');
      expect(writes).toHaveLength(1);
      expect(writes[0].pierre.access_token).toBe('renewed-access');
      expect(writes[0].pierre.refresh_token).toBe('renewed-refresh');
      expect(writes[0].pierre.saved_at).toBeGreaterThan(0);
    } finally {
      await closeServer(pierre.server);
    }
  });

  test('a refresh response without an access token leaves the stored session alone', async () => {
    const pierre = await startPierreStub({
      '/oauth2/validate-and-refresh': () => ({
        body: { status: 'refreshed', token_type: 'Bearer', expires_in: 3600 },
      }),
    });

    const provider = makeProvider({}, pierre.url);
    const writes = [];
    provider.secureStorage = recordingStorage(writes);
    provider.allStoredTokens = storedSession();

    try {
      await provider.validateAndRefreshToken('stale-access', 'rotating-refresh');

      expect(provider.allStoredTokens.pierre.access_token).toBe('stale-access');
      expect(provider.allStoredTokens.pierre.refresh_token).toBe('rotating-refresh');
      expect(writes).toHaveLength(0);
    } finally {
      await closeServer(pierre.server);
    }
  });
});

describe('tool-call recovery after an authentication error', () => {
  test('the retry carries the renewed access token and storage keeps the live refresh token', async () => {
    // The session is good when the turn starts and the access token is rejected mid-call;
    // the refresh that follows rotates both halves of the pair.
    const pierre = await startPierreStub({
      '/oauth2/validate-and-refresh': (callNumber) =>
        callNumber === 1
          ? { body: { status: 'valid', expires_in: 3600 } }
          : {
              body: {
                status: 'refreshed',
                access_token: jwtFor('user-1'),
                refresh_token: 'renewed-refresh',
                token_type: 'Bearer',
                expires_in: 3600,
              },
            },
    });

    const provider = makeProvider({}, pierre.url);
    const writes = [];
    provider.secureStorage = recordingStorage(writes);
    provider.allStoredTokens = storedSession();

    const bridge = new PierreMcpClient({
      mode: 'oauth',
      pierreServerUrl: pierre.url,
      oauthClientId: 'test-client',
      oauthClientSecret: 'test-secret',
    });
    bridge.log = () => {};
    await bridge.createMcpServer();
    bridge.oauthProvider = provider;

    // Stand-in for the Pierre MCP client. The real Streamable HTTP transport asks the
    // OAuth provider for tokens on every send, so this records the bearer each call
    // would have carried and rejects the first one the way an expired session does.
    const bearers = [];
    bridge.pierreClient = {
      callTool: async ({ name }) => {
        const tokens = await provider.tokens();
        bearers.push(tokens.access_token);
        if (bearers.length === 1) {
          const authError = new Error('HTTP 401 Unauthorized');
          authError.data = { authentication_failed: true };
          throw authError;
        }
        return {
          content: [{ type: 'text', text: `${name} ran as ${tokens.access_token}` }],
          isError: false,
        };
      },
    };

    try {
      const handler = bridge.mcpServer._requestHandlers.get('tools/call');
      const result = await handler(
        { method: 'tools/call', params: { name: 'get_activities', arguments: {} } },
        { signal: new AbortController().signal },
      );

      expect(bearers).toEqual(['stale-access', jwtFor('user-1')]);
      expect(result.isError).toBe(false);
      expect(result.content[0].text).toBe(`get_activities ran as ${jwtFor('user-1')}`);

      // The refresh token the server just retired is not what the keychain holds.
      expect(provider.allStoredTokens.pierre.refresh_token).toBe('renewed-refresh');
      expect(writes[writes.length - 1].pierre.refresh_token).toBe('renewed-refresh');

      // The refresh presented the stored token, not an already-rotated one.
      const refreshCalls = pierre.requests.filter(
        (r) => r.url === '/oauth2/validate-and-refresh',
      );
      expect(refreshCalls).toHaveLength(2);
      expect(JSON.parse(refreshCalls[1].body).refresh_token).toBe('rotating-refresh');
    } finally {
      await bridge.mcpServer.close();
      await closeServer(pierre.server);
    }
  });
});
