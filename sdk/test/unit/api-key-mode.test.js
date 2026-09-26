// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves api-key mode authenticates with the configured key and never with anything else
// ABOUTME: Drives the real bridge startup, tool calls and connect_provider against a scripted Dravr

const fs = require('fs');
const http = require('http');
const os = require('os');
const path = require('path');
const { EncryptedFileStorage, PierreMcpClient } = require('../../dist/index.js');
const { freePort } = require('./oauth-callback-harness.js');
const { complete } = require('../helpers/modern-dravr.js');

const API_KEY = 'pk_live_abcdefghijklmnopqrstuvwxyz012345';
const MINTED_URL = 'https://www.strava.com/oauth/authorize?client_id=1&state=minted-state';

/** A session an earlier OAuth sign-in left in storage, which api-key mode must never use. */
const STORED_SESSION = {
  pierre: {
    access_token: 'stored-oauth-session',
    refresh_token: 'stored-refresh',
    token_type: 'Bearer',
    expires_in: 3600,
    scope: 'fitness:read fitness:write profile:read profile:write',
    saved_at: Math.floor(Date.now() / 1000),
  },
  client_info: {
    client_id: 'stored-client',
    client_secret: 'stored-secret',
    redirect_uris: ['http://localhost:35535/oauth/callback'],
    scope: 'fitness:read fitness:write profile:read profile:write',
  },
};

/**
 * A scripted Dravr that authenticates the way the real one does: /mcp takes the API key
 * as a bearer, the REST initiation route takes it as the bare Authorization value. Every
 * request is recorded, so a test can assert both what was sent and what was never asked.
 */
function startDravr({ acceptKey = true, mintStatus = 200, mintRefusal = { error: 'unauthorized' } } = {}) {
  const seen = [];
  return new Promise((resolve) => {
    const server = http.createServer((req, res) => {
      let body = '';
      req.on('data', (chunk) => {
        body += chunk;
      });
      req.on('end', () => {
        const rpc = body ? JSON.parse(body) : undefined;
        seen.push({ method: req.method, url: req.url, headers: req.headers, rpc });
        const send = (status, json, headers = {}) => {
          res.writeHead(status, { 'Content-Type': 'application/json', ...headers });
          res.end(JSON.stringify(json));
        };

        if (req.method === 'GET' && req.url === '/api/oauth/mobile/init/strava') {
          if (req.headers.authorization !== API_KEY || mintStatus !== 200) {
            send(mintStatus === 200 ? 401 : mintStatus, mintRefusal);
            return;
          }
          send(200, { authorization_url: MINTED_URL, provider: 'strava', state: 'minted-state' });
          return;
        }

        if (req.method !== 'POST' || req.url !== '/mcp') {
          send(404, {});
          return;
        }
        if (!acceptKey || req.headers.authorization !== `Bearer ${API_KEY}`) {
          send(
            401,
            { jsonrpc: '2.0', id: null, error: { code: -32001, message: 'Unauthorized' } },
            { 'WWW-Authenticate': 'Bearer error="invalid_token"' },
          );
          return;
        }
        switch (rpc.method) {
          case 'server/discover':
            send(200, complete(rpc.id, {
              supportedVersions: ['2026-07-28'],
              capabilities: { tools: {} },
              serverInfo: { name: 'pierre-mcp-server', version: '0.0.0-test' },
            }));
            return;
          case 'tools/list':
            send(200, complete(rpc.id, {
              tools: [
                { name: 'get_athlete', description: 'Athlete profile', inputSchema: { type: 'object' } },
                { name: 'get_activities', description: 'Activities', inputSchema: { type: 'object' } },
              ],
            }));
            return;
          case 'tools/call':
            send(200, complete(rpc.id, {
              content: [{ type: 'text', text: `${rpc.params.name} ok` }],
              structuredContent: { name: rpc.params.name },
              isError: false,
            }));
            return;
          default:
            send(404, { jsonrpc: '2.0', id: rpc.id, error: { code: -32601, message: 'Method not found' } });
        }
      });
    });
    server.listen(0, '127.0.0.1', () => {
      resolve({
        seen,
        url: `http://127.0.0.1:${server.address().port}`,
        close: () =>
          new Promise((done) => {
            server.closeAllConnections?.();
            server.close(done);
          }),
      });
    });
  });
}

/**
 * The environment of one bridge start: a private home whose token file already holds an
 * OAuth session and client registration, the file store selected (CI) so no OS keychain
 * is touched, and the client's own logging silenced.
 */
async function isolatedHome() {
  const saved = { HOME: process.env.HOME, CI: process.env.CI };
  const home = fs.mkdtempSync(path.join(os.tmpdir(), 'pierre-api-key-'));
  process.env.HOME = home;
  process.env.CI = 'true';
  const consoleError = jest.spyOn(console, 'error').mockImplementation(() => {});
  await new EncryptedFileStorage(() => {}).saveTokens(STORED_SESSION);
  return {
    stored: () => new EncryptedFileStorage(() => {}).getTokens(),
    restore: () => {
      consoleError.mockRestore();
      for (const [name, value] of Object.entries(saved)) {
        if (value === undefined) {
          delete process.env[name];
        } else {
          process.env[name] = value;
        }
      }
      fs.rmSync(home, { recursive: true, force: true });
    },
  };
}

/** A bridge in api-key mode, started through its real connection path. */
async function startedBridge(dravr) {
  const bridge = new PierreMcpClient({
    mode: 'api-key',
    pierreServerUrl: dravr.url,
    apiKey: API_KEY,
    disableBrowser: true,
    callbackPort: await freePort(),
    // The bridge's connection deadlines are timers it leaves running after the race they
    // bound is won; short ones let the test process exit promptly.
    proactiveConnectionTimeoutMs: 2000,
    proactiveToolsListTimeoutMs: 2000,
    toolCallConnectionTimeoutMs: 2000,
  });
  const logs = [];
  bridge.log = (message) => logs.push(message);
  await bridge.createMcpServer();
  await bridge.initializePierreConnection();
  return {
    bridge,
    logs,
    handler: (method) => bridge.mcpServer._requestHandlers.get(method),
    cleanup: async () => {
      bridge.oauthProvider?.teardownAuthorizationFlow();
      await bridge.mcpServer.close();
    },
  };
}

describe('api-key mode', () => {
  let dravr;
  let env;

  beforeEach(async () => {
    env = await isolatedHome();
  });

  afterEach(async () => {
    if (dravr) {
      await dravr.close();
      dravr = undefined;
    }
    env.restore();
  });

  test('sends the key as the bearer on every /mcp request and asks for no OAuth at all', async () => {
    dravr = await startDravr();
    const wired = await startedBridge(dravr);
    try {
      expect(wired.bridge.cachedTools.tools.map((t) => t.name)).toEqual(['get_athlete', 'get_activities']);

      const result = await wired.handler('tools/call')(
        { method: 'tools/call', params: { name: 'get_athlete', arguments: {} } },
        { signal: new AbortController().signal },
      );
      expect(result.content[0].text).toBe('get_athlete ok');

      expect(dravr.seen.map((r) => `${r.method} ${r.url} ${r.rpc?.method}`)).toEqual([
        'POST /mcp server/discover',
        'POST /mcp tools/list',
        'POST /mcp tools/call',
      ]);
      for (const request of dravr.seen) {
        expect(request.headers.authorization).toBe(`Bearer ${API_KEY}`);
      }
      expect(wired.bridge.getClientSideTokenStatus().pierre).toBe(true);

      // The stored session was neither presented, validated nor cleared: it is not this
      // mode's to touch.
      const stored = await env.stored();
      expect(stored.pierre.access_token).toBe('stored-oauth-session');
      expect(stored.client_info.client_id).toBe('stored-client');
    } finally {
      await wired.cleanup();
    }
  });

  test('a key Dravr refuses is reported as refused, once, and no sign-in starts in its place', async () => {
    dravr = await startDravr({ acceptKey: false });
    const wired = await startedBridge(dravr);
    try {
      expect(wired.bridge.pierreClient).toBeNull();

      const toolResult = await wired.handler('tools/call')(
        { method: 'tools/call', params: { name: 'get_athlete', arguments: {} } },
        { signal: new AbortController().signal },
      );
      expect(toolResult.isError).toBe(true);
      expect(toolResult.content[0].text).toMatch(/Dravr refused the configured API key/);

      const connectResult = await wired.handler('tools/call')(
        { method: 'tools/call', params: { name: 'connect_to_dravr', arguments: {} } },
        { signal: new AbortController().signal },
      );
      expect(connectResult.isError).toBe(true);
      expect(connectResult.content[0].text).toMatch(/Dravr refused the configured API key/);

      // One discovery per attempt - startup, the tool call, connect_to_dravr - each with
      // the key, and nothing sent to any OAuth endpoint.
      expect(dravr.seen.map((r) => `${r.method} ${r.url} ${r.rpc?.method}`)).toEqual([
        'POST /mcp server/discover',
        'POST /mcp server/discover',
        'POST /mcp server/discover',
      ]);
      for (const request of dravr.seen) {
        expect(request.headers.authorization).toBe(`Bearer ${API_KEY}`);
      }
      expect(wired.logs.some((line) => /authorization flow/i.test(line))).toBe(false);
    } finally {
      await wired.cleanup();
    }
  });

  test('connect_provider has Dravr mint the authorization page for the key and opens that page', async () => {
    dravr = await startDravr();
    const wired = await startedBridge(dravr);
    const waits = [];
    wired.bridge.oauthProvider.waitForProviderOAuth = async (provider, ms) => {
      waits.push({ provider, ms });
    };
    try {
      const result = await wired.handler('tools/call')(
        { method: 'tools/call', params: { name: 'connect_provider', arguments: { provider: 'strava' } } },
        { signal: new AbortController().signal },
      );

      expect(result.isError).toBe(false);
      expect(result.content[0].text).toMatch(/Strava connected successfully/);

      const mint = dravr.seen.find((r) => r.url === '/api/oauth/mobile/init/strava');
      expect(mint.method).toBe('GET');
      // REST reads the key as the whole Authorization value; a Bearer scheme there is a
      // session token.
      expect(mint.headers.authorization).toBe(API_KEY);
      // The flow starts with the callback listener's per-flow token, the one Dravr must
      // present on its completion POST for the listener to accept the provider tokens.
      const listenerToken = wired.bridge.oauthProvider.callbackAuthToken;
      expect(listenerToken).toMatch(/^[0-9a-f]{64}$/);
      expect(mint.headers['x-callback-token']).toBe(listenerToken);
      // It rides a header only: no URL, and no page the browser opens, carries it.
      expect(wired.logs.some((line) => line.includes(listenerToken))).toBe(false);
      expect(wired.logs).toContain(`OAuth URL: ${MINTED_URL}`);
      expect(waits).toEqual([{ provider: 'strava', ms: 55000 }]);
    } finally {
      await wired.cleanup();
    }
  });

  test('connect_provider reads a notice refusal as where to accept it, and opens no page', async () => {
    dravr = await startDravr({
      mintStatus: 400,
      mintRefusal: {
        code: 'InvalidInput',
        message: 'Connecting Strava requires accepting the account notice first',
        details: { action: 'accept_provider_notice', provider: 'strava' },
      },
    });
    const wired = await startedBridge(dravr);
    const waits = [];
    wired.bridge.oauthProvider.waitForProviderOAuth = async (provider) => {
      waits.push(provider);
    };
    try {
      const result = await wired.handler('tools/call')(
        { method: 'tools/call', params: { name: 'connect_provider', arguments: { provider: 'strava' } } },
        { signal: new AbortController().signal },
      );

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toMatch(/Connecting STRAVA needs your authorization first/);
      expect(result.content[0].text).toMatch(/Open the Dravr app, go to Connections/);
      expect(wired.logs.some((line) => line.startsWith('OAuth URL:'))).toBe(false);
      expect(waits).toEqual([]);
    } finally {
      await wired.cleanup();
    }
  });

  test('connect_provider reports a refused mint instead of opening any page', async () => {
    dravr = await startDravr({ mintStatus: 403 });
    const wired = await startedBridge(dravr);
    const waits = [];
    wired.bridge.oauthProvider.waitForProviderOAuth = async (provider) => {
      waits.push(provider);
    };
    try {
      const result = await wired.handler('tools/call')(
        { method: 'tools/call', params: { name: 'connect_provider', arguments: { provider: 'strava' } } },
        { signal: new AbortController().signal },
      );

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toMatch(
        /Dravr refused to start strava authorization for the configured API key \(HTTP 403\)/,
      );
      expect(wired.logs.some((line) => line.startsWith('OAuth URL:'))).toBe(false);
      expect(waits).toEqual([]);
    } finally {
      await wired.cleanup();
    }
  });
});
