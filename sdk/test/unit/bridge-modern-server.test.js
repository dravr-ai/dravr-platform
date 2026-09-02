// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves the bridge reaches Dravr as a 2026-07-28 client: discovery, listing, calls, tasks
// ABOUTME: Runs the real host-side handlers against a scripted Dravr endpoint, no Rust server needed

const http = require('http');
const { PierreMcpClient, MCP_PROTOCOL_VERSION, TASKS_EXTENSION_ID } = require('../../dist/index.js');
const { startProvider, stopProvider } = require('./oauth-callback-harness.js');

const META_CAPABILITIES = 'io.modelcontextprotocol/clientCapabilities';
const META_VERSION = 'io.modelcontextprotocol/protocolVersion';

/** An access token the bridge can decode a user id out of. */
function jwtFor(subject) {
  const payload = Buffer.from(JSON.stringify({ sub: subject })).toString('base64');
  return `header.${payload}.signature`;
}

const complete = (id, result) => ({ jsonrpc: '2.0', id, result: { resultType: 'complete', ...result } });

/**
 * A scripted Dravr: a modern /mcp that challenges without a bearer, discovers with the
 * Tasks extension, lists two tools, answers `get_athlete` inline and `get_activities`
 * with a task handle that completes on the second poll.
 */
function startDravr({ tasks = true } = {}) {
  const seen = [];
  let polls = 0;
  return new Promise((resolve) => {
    const server = http.createServer((req, res) => {
      let body = '';
      req.on('data', (chunk) => {
        body += chunk;
      });
      req.on('end', () => {
        const rpc = JSON.parse(body);
        seen.push({ headers: req.headers, rpc });
        const send = (status, json, headers = {}) => {
          res.writeHead(status, { 'Content-Type': 'application/json', ...headers });
          res.end(JSON.stringify(json));
        };
        if (!req.headers.authorization) {
          send(
            401,
            { jsonrpc: '2.0', id: null, error: { code: -32001, message: 'Unauthorized' } },
            { 'WWW-Authenticate': 'Bearer resource_metadata="http://dravr.test/.well-known/oauth-protected-resource"' },
          );
          return;
        }
        const declaresTasks = !!rpc.params?._meta?.[META_CAPABILITIES]?.extensions?.[TASKS_EXTENSION_ID];
        switch (rpc.method) {
          case 'server/discover':
            send(200, complete(rpc.id, {
              supportedVersions: ['2026-07-28', '2025-11-25'],
              capabilities: tasks ? { tools: {}, extensions: { [TASKS_EXTENSION_ID]: {} } } : { tools: {} },
              serverInfo: { name: 'pierre-mcp-server', version: '0.0.0-test' },
            }));
            return;
          case 'tools/list':
            send(200, complete(rpc.id, {
              tools: [
                { name: 'get_athlete', description: 'Athlete profile', inputSchema: { type: 'object' } },
                { name: 'get_activities', description: 'Activities', inputSchema: { type: 'object' } },
              ],
              ttlMs: 60000,
              cacheScope: 'private',
            }));
            return;
          case 'tools/call':
            if (rpc.params.name === 'get_activities' && declaresTasks) {
              send(200, { jsonrpc: '2.0', id: rpc.id, result: { resultType: 'task', taskId: 'backfill-1', status: 'working', createdAt: 't0', lastUpdatedAt: 't0', ttlMs: 60000, pollIntervalMs: 10 } });
              return;
            }
            send(200, complete(rpc.id, { content: [{ type: 'text', text: `${rpc.params.name} ok` }], structuredContent: { name: rpc.params.name }, isError: false }));
            return;
          case 'tasks/get':
            polls += 1;
            send(200, complete(rpc.id, {
              taskId: 'backfill-1',
              status: polls < 2 ? 'working' : 'completed',
              statusMessage: polls < 2 ? 'scraping 2019' : undefined,
              createdAt: 't0',
              lastUpdatedAt: 't1',
              ttlMs: 60000,
              pollIntervalMs: 10,
              ...(polls < 2 ? {} : { result: { resultType: 'complete', content: [{ type: 'text', text: '412 activities' }], isError: false } }),
            }));
            return;
          case 'resources/read':
            send(200, complete(rpc.id, { contents: [{ uri: rpc.params.uri, mimeType: 'text/plain', text: 'profile' }] }));
            return;
          default:
            send(404, { jsonrpc: '2.0', id: rpc.id, error: { code: -32601, message: `Method not found: ${rpc.method}` } });
        }
      });
    });
    server.listen(0, '127.0.0.1', () => {
      resolve({
        seen,
        url: `http://127.0.0.1:${server.address().port}`,
        close: () => new Promise((done) => server.close(done)),
      });
    });
  });
}

/** A bridge pointed at the scripted Dravr, with the session it is given and no browser. */
async function wiredBridge(dravr, { token } = {}) {
  const { provider } = await startProvider({ disableBrowser: true }, dravr.url);
  if (token) {
    provider.savedTokens = {
      access_token: token,
      token_type: 'Bearer',
      expires_in: 3600,
      scope: 'read:fitness write:fitness',
    };
  }
  const bridge = new PierreMcpClient({
    mode: 'oauth',
    pierreServerUrl: dravr.url,
    oauthClientId: 'test-client',
    oauthClientSecret: 'test-secret',
    disableBrowser: true,
  });
  bridge.log = () => {};
  await bridge.createMcpServer();
  bridge.oauthProvider = provider;
  bridge.mcpUrl = `${dravr.url}/mcp`;
  return {
    bridge,
    provider,
    handler: (method) => bridge.mcpServer._requestHandlers.get(method),
    cleanup: async () => {
      stopProvider(provider);
      await bridge.mcpServer.close();
    },
  };
}

describe('the bridge speaks 2026-07-28 to Dravr', () => {
  let dravr;
  afterEach(async () => {
    if (dravr) {
      await dravr.close();
    }
  });

  test('discovers instead of initializing, and lists and calls tools with the session bearer', async () => {
    dravr = await startDravr();
    const wired = await wiredBridge(dravr, { token: jwtFor('user-1') });
    try {
      await wired.bridge.attemptConnection();
      expect(wired.bridge.pierreClient.serverSupportsTasks()).toBe(true);

      const listing = await wired.handler('tools/list')({ method: 'tools/list', params: {} });
      expect(listing.tools.map((t) => t.name)).toEqual(['get_athlete', 'get_activities']);
      // The host gets tools; the listing's cache hints are a promise to the bridge, not to it.
      expect(listing.ttlMs).toBeUndefined();
      expect(listing.cacheScope).toBeUndefined();

      const result = await wired.handler('tools/call')(
        { method: 'tools/call', params: { name: 'get_athlete', arguments: { provider: 'strava' } } },
        { signal: new AbortController().signal },
      );
      expect(result.content[0].text).toBe('get_athlete ok');
      expect(result.resultType).toBeUndefined();

      const methods = dravr.seen.map((r) => r.rpc.method);
      expect(methods).toEqual(['server/discover', 'tools/list', 'tools/call']);
      expect(methods).not.toContain('initialize');
      expect(methods).not.toContain('ping');
      for (const request of dravr.seen) {
        expect(request.headers.authorization).toBe(`Bearer ${jwtFor('user-1')}`);
        expect(request.headers['mcp-protocol-version']).toBe(MCP_PROTOCOL_VERSION);
        expect(request.headers['mcp-method']).toBe(request.rpc.method);
        expect(request.rpc.params._meta[META_VERSION]).toBe(MCP_PROTOCOL_VERSION);
      }
      expect(dravr.seen[2].headers['mcp-name']).toBe('get_athlete');
    } finally {
      await wired.cleanup();
    }
  });

  test('a call the server answers with a task handle comes back to the host as the tool result', async () => {
    dravr = await startDravr();
    const wired = await wiredBridge(dravr, { token: jwtFor('user-1') });
    try {
      await wired.bridge.attemptConnection();
      const notifications = [];
      const result = await wired.handler('tools/call')(
        { method: 'tools/call', params: { name: 'get_activities', arguments: { after: 1546300800 } } },
        {
          signal: new AbortController().signal,
          _meta: { progressToken: 'host-7' },
          sendNotification: async (n) => {
            notifications.push(n);
          },
        },
      );

      expect(result).toEqual({ content: [{ type: 'text', text: '412 activities' }], isError: false });
      expect(dravr.seen.map((r) => r.rpc.method)).toEqual(['server/discover', 'tools/call', 'tasks/get', 'tasks/get']);
      // The call declared the extension; that is what let the server hand back a handle.
      expect(dravr.seen[1].rpc.params._meta[META_CAPABILITIES]).toEqual({ extensions: { [TASKS_EXTENSION_ID]: {} } });

      // The host offered a progress token, so it learned the call was running as a task
      // and heard each poll, with a count that only rises.
      expect(notifications.map((n) => n.method)).toEqual(['notifications/progress', 'notifications/progress', 'notifications/progress']);
      expect(notifications.map((n) => n.params.progress)).toEqual([1, 2, 3]);
      expect(notifications.every((n) => n.params.progressToken === 'host-7')).toBe(true);
      expect(notifications[0].params.message).toMatch(/task backfill-1/);
      expect(notifications[1].params.message).toBe('scraping 2019');
    } finally {
      await wired.cleanup();
    }
  });

  test('a host without a progress token is told nothing mid-call', async () => {
    dravr = await startDravr();
    const wired = await wiredBridge(dravr, { token: jwtFor('user-1') });
    try {
      await wired.bridge.attemptConnection();
      const notifications = [];
      const result = await wired.handler('tools/call')(
        { method: 'tools/call', params: { name: 'get_activities', arguments: {} } },
        { signal: new AbortController().signal, sendNotification: async (n) => notifications.push(n) },
      );
      expect(result.content[0].text).toBe('412 activities');
      expect(notifications).toEqual([]);
    } finally {
      await wired.cleanup();
    }
  });

  test('without a session the challenge is accepted as the server being there, and the connect tool stands in', async () => {
    dravr = await startDravr();
    const wired = await wiredBridge(dravr);
    try {
      await wired.bridge.attemptConnection();
      expect(wired.bridge.pierreClient).toBeTruthy();
      expect(dravr.seen.map((r) => r.rpc.method)).toEqual(['server/discover']);
      expect(dravr.seen[0].headers.authorization).toBeUndefined();

      const listing = await wired.handler('tools/list')({ method: 'tools/list', params: {} });
      expect(listing.tools.map((t) => t.name)).toEqual(['connect_to_dravr']);
    } finally {
      await wired.cleanup();
    }
  });

  test('a session the server rejects is dropped, and discovery continues without it', async () => {
    dravr = await startDravr();
    // The scripted Dravr accepts any bearer, so a rejection is scripted at the source:
    // the first discovery goes out authenticated and is refused.
    let refusals = 0;
    const accepting = dravr;
    const refusing = await new Promise((resolve) => {
      const server = http.createServer((req, res) => {
        let body = '';
        req.on('data', (c) => {
          body += c;
        });
        req.on('end', () => {
          if (req.headers.authorization && refusals === 0) {
            refusals += 1;
            res.writeHead(401, { 'Content-Type': 'application/json', 'WWW-Authenticate': 'Bearer error="invalid_token"' });
            res.end(JSON.stringify({ jsonrpc: '2.0', id: null, error: { code: -32001, message: 'Unauthorized' } }));
            return;
          }
          // Everything else goes to the accepting Dravr's logic by re-posting.
          fetch(`${accepting.url}/mcp`, { method: 'POST', headers: req.headers, body }).then(async (upstream) => {
            res.writeHead(upstream.status, Object.fromEntries(upstream.headers));
            res.end(await upstream.text());
          });
        });
      });
      server.listen(0, '127.0.0.1', () => resolve({ server, url: `http://127.0.0.1:${server.address().port}` }));
    });
    const wired = await wiredBridge({ url: refusing.url }, { token: jwtFor('user-1') });
    const invalidated = [];
    wired.provider.invalidateCredentials = async (scope) => {
      invalidated.push(scope);
      wired.provider.savedTokens = undefined;
    };
    try {
      await wired.bridge.attemptConnection();
      expect(invalidated).toEqual(['tokens']);
      expect(refusals).toBe(1);
      // The retry went out without the dead session and settled on the challenge.
      expect(wired.bridge.pierreClient).toBeTruthy();
    } finally {
      await wired.cleanup();
      await new Promise((done) => refusing.server.close(done));
    }
  });

  test('resources/read is forwarded as a modern request and the discriminator is not passed to the host', async () => {
    dravr = await startDravr();
    const wired = await wiredBridge(dravr, { token: jwtFor('user-1') });
    try {
      await wired.bridge.attemptConnection();
      const result = await wired.handler('resources/read')({ method: 'resources/read', params: { uri: 'dravr://athlete/profile' } });
      expect(result).toEqual({ contents: [{ uri: 'dravr://athlete/profile', mimeType: 'text/plain', text: 'profile' }] });
      const read = dravr.seen[dravr.seen.length - 1];
      expect(read.headers['mcp-name']).toBe('dravr://athlete/profile');
    } finally {
      await wired.cleanup();
    }
  });

  test('a server that never advertised the extension gets calls without it, and they answer inline', async () => {
    dravr = await startDravr({ tasks: false });
    const wired = await wiredBridge(dravr, { token: jwtFor('user-1') });
    try {
      await wired.bridge.attemptConnection();
      expect(wired.bridge.pierreClient.serverSupportsTasks()).toBe(false);
      const result = await wired.handler('tools/call')(
        { method: 'tools/call', params: { name: 'get_activities', arguments: {} } },
        { signal: new AbortController().signal },
      );
      expect(result.content[0].text).toBe('get_activities ok');
      expect(dravr.seen[1].rpc.params._meta[META_CAPABILITIES]).toEqual({});
    } finally {
      await wired.cleanup();
    }
  });
});
