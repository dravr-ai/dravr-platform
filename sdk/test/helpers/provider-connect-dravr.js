// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A scripted Dravr and an oauth-mode bridge wired to it, for suites that drive connect_provider
// ABOUTME: Shared by the poll suite and the browser-launch injection suite, so both drive one fake

const http = require('http');
const { PierreMcpClient } = require('../../dist/index.js');
const { makeProvider, stopProvider } = require('../unit/oauth-callback-harness.js');
const { complete } = require('./modern-dravr.js');

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
 * /mcp answers discovery, `connect_provider` with the provider page it mints
 * (`providerPage`), and `get_connection_status`, each read with the next of `states` and
 * the last one after that; a state of 'error' fails that read. Every request is recorded.
 */
function startDravr(states, providerPage = PROVIDER_PAGE) {
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
              authorization_url: providerPage,
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
 * The browser stays disabled unless a suite that captures the launch asks for it.
 */
async function wiredBridge(dravr, accessToken = jwtFor('user-1'), { disableBrowser = true } = {}) {
  const provider = makeProvider({ disableBrowser }, dravr.url);
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
    disableBrowser,
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

module.exports = { PROVIDER_PAGE, fastPoll, jwtFor, startDravr, wiredBridge };
