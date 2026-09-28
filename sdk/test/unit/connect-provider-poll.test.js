// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: connect_provider has Dravr's connect_provider MCP tool mint the page, then polls get_connection_status
// ABOUTME: Drives the real oauth-mode bridge (a delegated grant) and MCP client against a scripted Dravr, end to end

const http = require('http');
const { PierreMcpClient } = require('../../dist/index.js');
const { makeProvider, stopProvider } = require('./oauth-callback-harness.js');
const { complete } = require('../helpers/modern-dravr.js');

const PROVIDER_PAGE = 'https://www.strava.com/oauth/authorize?client_id=1&state=minted';

/** An access token the bridge can decode a user id out of. */
function jwtFor(subject) {
  const payload = Buffer.from(JSON.stringify({ sub: subject })).toString('base64');
  return `header.${payload}.signature`;
}

/** What get_connection_status answers for Strava in each scripted state. */
function stravaStatus(state) {
  return {
    provider: 'strava',
    status: state,
    connected: state === 'connected' || state === 'needs_reauth',
    needs_reauth: state === 'needs_reauth',
    backend: state === 'disconnected' ? 'none' : 'oauth',
  };
}

/** The sentence Dravr's REST routes refuse a delegated OAuth grant with. */
const DELEGATION_REFUSAL =
  'This access token was delegated to an application and is accepted only by the MCP and A2A endpoints';

/**
 * A scripted Dravr that treats an oauth-mode bridge's token as the delegated grant it
 * is. Every REST route refuses it with the delegation 403, the launch routes included.
 * /mcp answers discovery, `connect_provider` with the provider page it mints, and
 * `get_connection_status`, each read with the next of `states` and the last one after
 * that; a state of 'error' fails that read. Every request is recorded.
 */
function startDravr(states) {
  const seen = [];
  let reads = 0;
  return new Promise((resolve) => {
    const server = http.createServer((req, res) => {
      let body = '';
      req.on('data', (chunk) => {
        body += chunk;
      });
      req.on('end', () => {
        const rpc = body ? JSON.parse(body) : undefined;
        seen.push({ method: req.method, url: req.url, headers: req.headers, rpc });
        const send = (status, json) => {
          res.writeHead(status, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify(json));
        };

        if (req.method !== 'POST' || req.url !== '/mcp') {
          send(403, { code: 'PermissionDenied', message: DELEGATION_REFUSAL });
          return;
        }
        if (rpc.method === 'server/discover') {
          send(200, complete(rpc.id, {
            supportedVersions: ['2026-07-28'],
            capabilities: { tools: {} },
            serverInfo: { name: 'pierre-mcp-server', version: '0.0.0-test' },
          }));
          return;
        }
        if (rpc.method === 'tools/call' && rpc.params.name === 'connect_provider') {
          send(200, complete(rpc.id, {
            content: [{ type: 'text', text: 'pending_authorization' }],
            structuredContent: {
              provider: rpc.params.arguments.provider,
              authorization_url: PROVIDER_PAGE,
              state: 'minted',
              instructions: 'Visit the authorization URL',
              expires_in_minutes: 10,
              status: 'pending_authorization',
            },
            isError: false,
          }));
          return;
        }
        if (rpc.method === 'tools/call' && rpc.params.name === 'get_connection_status') {
          const state = states[Math.min(reads, states.length - 1)];
          reads += 1;
          if (state === 'error') {
            send(500, {
              jsonrpc: '2.0',
              id: rpc.id,
              error: { code: -32603, message: 'status store unavailable' },
            });
            return;
          }
          send(200, complete(rpc.id, {
            content: [{ type: 'text', text: state }],
            structuredContent: stravaStatus(state),
            isError: false,
          }));
          return;
        }
        send(404, { jsonrpc: '2.0', id: rpc.id, error: { code: -32601, message: 'Method not found' } });
      });
    });
    server.listen(0, '127.0.0.1', () => {
      resolve({
        seen,
        url: `http://127.0.0.1:${server.address().port}`,
        statusReads: () => seen.filter((r) => r.rpc?.params?.name === 'get_connection_status'),
        mints: () => seen.filter((r) => r.rpc?.params?.name === 'connect_provider'),
        restCalls: () => seen.filter((r) => r.method !== 'POST' || r.url !== '/mcp'),
        close: () =>
          new Promise((done) => {
            server.closeAllConnections();
            server.close(done);
          }),
      });
    });
  });
}

/** The grant an oauth-mode bridge holds: every delegable scope, never the self grant. */
const DELEGATED_SCOPE = 'fitness:read fitness:write profile:read profile:write';

/**
 * An oauth-mode bridge signed in to the scripted Dravr through its real MCP client with
 * a delegated grant, and no callback listener bound: nothing in a provider flow needs one.
 */
async function wiredBridge(dravr, accessToken = jwtFor('user-1')) {
  const provider = makeProvider({ disableBrowser: true }, dravr.url);
  provider.savedTokens = {
    access_token: accessToken,
    token_type: 'Bearer',
    expires_in: 3600,
    scope: DELEGATED_SCOPE,
  };
  const bridge = new PierreMcpClient({
    mode: 'oauth',
    pierreServerUrl: dravr.url,
    oauthClientId: 'test-client',
    oauthClientSecret: 'test-secret',
    disableBrowser: true,
  });
  const logs = [];
  bridge.log = (message) => logs.push(message);
  await bridge.createMcpServer();
  bridge.oauthProvider = provider;
  bridge.mcpUrl = `${dravr.url}/mcp`;
  await bridge.attemptConnection();
  return {
    bridge,
    provider,
    logs,
    connect: (extra = { signal: new AbortController().signal }) =>
      bridge.mcpServer._requestHandlers.get('tools/call')(
        { method: 'tools/call', params: { name: 'connect_provider', arguments: { provider: 'strava' } } },
        extra,
      ),
    cleanup: async () => {
      stopProvider(provider);
      await bridge.mcpServer.close();
    },
  };
}

/**
 * Keeps the real poll on a clock a test can afford, recording the budget each wait was
 * handed: `budgetMs` replaces the host's budget when given, and reads come every 10ms
 * instead of every two seconds.
 */
function fastPoll(bridge, budgetMs) {
  const waits = [];
  const realWait = bridge.waitForProviderConnection.bind(bridge);
  bridge.waitForProviderConnection = (provider, ms, signal) => {
    waits.push({ provider, ms });
    return realWait(provider, budgetMs ?? ms, signal, 10);
  };
  return waits;
}

const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

describe('connect_provider mints the page over MCP and polls Dravr for the connection', () => {
  let dravr;
  afterEach(async () => {
    if (dravr) {
      await dravr.close();
      dravr = undefined;
    }
  });

  test('a delegated grant completes end to end: the MCP tool mints the page, no REST launch is asked', async () => {
    dravr = await startDravr(['disconnected', 'disconnected', 'disconnected', 'connected']);
    const wired = await wiredBridge(dravr);
    const waits = fastPoll(wired.bridge);
    try {
      const result = await wired.connect();

      expect(result.isError).toBe(false);
      expect(result.content[0].text).toMatch(/^Strava connected successfully!/);
      expect(wired.logs).toContain(`OAuth URL: ${PROVIDER_PAGE}`);
      // The host's own budget, unchanged by the poll.
      expect(waits).toEqual([{ provider: 'strava', ms: 55000 }]);

      // One read before the page opened, then reads until the fourth said connected.
      const reads = dravr.statusReads();
      expect(reads).toHaveLength(4);
      for (const read of reads) {
        expect(read.rpc.params.arguments).toEqual({ provider: 'strava' });
        expect(read.headers.authorization).toBe(`Bearer ${jwtFor('user-1')}`);
      }

      // The flow starts on Dravr's connect_provider tool, over the same MCP session and
      // bearer as the reads: the tool takes the athlete from the credential, so no user id
      // is sent. The page opened is the one the tool minted.
      const mints = dravr.mints();
      expect(mints).toHaveLength(1);
      expect(mints[0].rpc.params.arguments).toEqual({ provider: 'strava' });
      expect(mints[0].headers.authorization).toBe(`Bearer ${jwtFor('user-1')}`);
      expect(mints[0].headers['x-callback-token']).toBeUndefined();
      expect(wired.logs).toContain(`Opened strava OAuth in browser: ${PROVIDER_PAGE}`);

      // Every request went to /mcp: no REST launch route, which refuses a delegated
      // grant, was asked. Nothing to call this machine back with was sent, and no
      // listener is bound for it.
      expect(dravr.restCalls()).toEqual([]);
      expect(dravr.seen.map((r) => r.rpc?.params?.name ?? r.rpc?.method)).toEqual([
        'server/discover',
        'get_connection_status',
        'connect_provider',
        'get_connection_status',
        'get_connection_status',
        'get_connection_status',
      ]);
      expect(wired.provider.callbackServer).toBeUndefined();
    } finally {
      await wired.cleanup();
    }
  });

  test('a session token the bridge cannot read still starts the flow: the credential names the athlete', async () => {
    dravr = await startDravr(['disconnected', 'connected']);
    const wired = await wiredBridge(dravr, 'opaque-session-token');
    fastPoll(wired.bridge);
    try {
      const result = await wired.connect();

      expect(result.isError).toBe(false);
      expect(result.content[0].text).toMatch(/^Strava connected successfully!/);
      const mints = dravr.mints();
      expect(mints).toHaveLength(1);
      expect(mints[0].headers.authorization).toBe('Bearer opaque-session-token');
      expect(dravr.restCalls()).toEqual([]);
    } finally {
      await wired.cleanup();
    }
  });

  test('a failed status read is asked again rather than taken for an outcome', async () => {
    dravr = await startDravr(['disconnected', 'error', 'connected']);
    const wired = await wiredBridge(dravr);
    fastPoll(wired.bridge);
    try {
      const result = await wired.connect();

      expect(result.isError).toBe(false);
      expect(result.content[0].text).toMatch(/^Strava connected successfully!/);
      expect(dravr.statusReads()).toHaveLength(3);
      expect(wired.logs.some((line) => /Connection status read failed, asking again/.test(line))).toBe(true);
    } finally {
      await wired.cleanup();
    }
  });

  test('a connection that needs reauthorizing is authorized again, and only a usable one ends the wait', async () => {
    dravr = await startDravr(['needs_reauth', 'needs_reauth', 'connected']);
    const wired = await wiredBridge(dravr);
    fastPoll(wired.bridge);
    try {
      const result = await wired.connect();

      expect(result.content[0].text).toMatch(/^Strava connected successfully!/);
      expect(dravr.mints()).toHaveLength(1);
      expect(dravr.statusReads()).toHaveLength(3);
    } finally {
      await wired.cleanup();
    }
  });

  test('times out cleanly: the budget spent is reported as pending and no read outlives the call', async () => {
    dravr = await startDravr(['disconnected']);
    const wired = await wiredBridge(dravr);
    fastPoll(wired.bridge, 150);
    try {
      const startedAt = Date.now();
      const result = await wired.connect();
      const elapsed = Date.now() - startedAt;

      expect(result.isError).toBe(false);
      expect(result.content[0].text).toMatch(/STRAVA authorization is still open in your browser and is not confirmed yet/);
      expect(result.content[0].text).toMatch(/run connect_provider again/);
      expect(result.content[0].text).not.toMatch(/connected successfully/i);
      expect(elapsed).toBeLessThan(2000);

      const readsAtReturn = dravr.statusReads().length;
      // The pre-check plus several polls inside a 150ms budget read every 10ms.
      expect(readsAtReturn).toBeGreaterThan(3);
      await pause(100);
      expect(dravr.statusReads()).toHaveLength(readsAtReturn);
    } finally {
      await wired.cleanup();
    }
  });

  test('the host cancelling the request ends the poll at once', async () => {
    dravr = await startDravr(['disconnected']);
    const wired = await wiredBridge(dravr);
    fastPoll(wired.bridge);
    const controller = new AbortController();
    try {
      const pending = wired.connect({ signal: controller.signal });
      await pause(60);
      controller.abort();
      const result = await pending;

      expect(result.isError).toBe(false);
      expect(result.content[0].text).toMatch(/Stopped waiting for STRAVA authorization/);

      const readsAtReturn = dravr.statusReads().length;
      await pause(80);
      expect(dravr.statusReads()).toHaveLength(readsAtReturn);
    } finally {
      await wired.cleanup();
    }
  });

  test('a provider already connected opens no page and polls nothing', async () => {
    dravr = await startDravr(['connected']);
    const wired = await wiredBridge(dravr);
    const waits = fastPoll(wired.bridge);
    try {
      const result = await wired.connect();

      expect(result.isError).toBe(false);
      expect(result.content[0].text).toMatch(/^Already connected to STRAVA!/);
      expect(dravr.mints()).toHaveLength(0);
      expect(dravr.restCalls()).toEqual([]);
      expect(dravr.statusReads()).toHaveLength(1);
      expect(waits).toEqual([]);
    } finally {
      await wired.cleanup();
    }
  });
});
