// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: connect_provider learns a provider flow's outcome by polling Dravr's get_connection_status
// ABOUTME: Drives the real bridge and MCP client against a scripted Dravr: connected, budget spent, cancelled

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

/**
 * A scripted Dravr. The initiate route answers the bridge's start with a 302 to the
 * provider's page, and /mcp answers discovery and `get_connection_status`, each read with
 * the next of `states` and the last one after that. A state of 'error' fails that read.
 * Every request is recorded.
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

        if (req.method === 'GET' && req.url === '/api/oauth/auth/strava/user-1') {
          res.writeHead(302, { Location: PROVIDER_PAGE });
          res.end();
          return;
        }
        if (req.method !== 'POST' || req.url !== '/mcp') {
          send(404, {});
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
        starts: () => seen.filter((r) => r.method === 'GET'),
        close: () =>
          new Promise((done) => {
            server.closeAllConnections();
            server.close(done);
          }),
      });
    });
  });
}

/**
 * A bridge signed in to the scripted Dravr through its real MCP client, with no callback
 * listener bound: nothing in a provider flow needs one.
 */
async function wiredBridge(dravr) {
  const provider = makeProvider({ disableBrowser: true }, dravr.url);
  provider.savedTokens = {
    access_token: jwtFor('user-1'),
    token_type: 'Bearer',
    expires_in: 3600,
    scope: 'fitness:read fitness:write profile:read profile:write',
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

describe('connect_provider polls Dravr for the connection', () => {
  let dravr;
  afterEach(async () => {
    if (dravr) {
      await dravr.close();
      dravr = undefined;
    }
  });

  test('resolves connected once the poll reads connected, with nothing sent back to this machine', async () => {
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

      // The flow starts with the session bearer and nothing to call this machine back
      // with, and no listener is bound for it.
      const starts = dravr.starts();
      expect(starts).toHaveLength(1);
      expect(starts[0].headers.authorization).toBe(`Bearer ${jwtFor('user-1')}`);
      expect(starts[0].headers['x-callback-token']).toBeUndefined();
      expect(wired.provider.callbackServer).toBeUndefined();
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
      expect(dravr.starts()).toHaveLength(1);
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
      expect(dravr.starts()).toHaveLength(0);
      expect(dravr.statusReads()).toHaveLength(1);
      expect(waits).toEqual([]);
    } finally {
      await wired.cleanup();
    }
  });
});
