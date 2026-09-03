// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Wire-level tests for the stateless 2026-07-28 client against a scripted MCP endpoint
// ABOUTME: Asserts per-request _meta and routing headers, JSON and SSE replies, and Tasks polling

const { buildSync } = require('esbuild');
const { mkdtempSync, rmSync } = require('fs');
const { join } = require('path');
const { tmpdir } = require('os');
const { complete, startEndpoint } = require('../helpers/modern-dravr.js');

const SRC = join(__dirname, '..', '..', 'src', 'mcp-http-client.ts');
const BUILD_DIR = mkdtempSync(join(tmpdir(), 'pierre-mcp-http-client-'));

let McpHttpClient;
let McpHttpError;
let McpRpcError;
let McpProtocolError;
let McpTaskCancelledError;
let encodeHeaderValue;
let MCP_PROTOCOL_VERSION;
let TASKS_EXTENSION_ID;

beforeAll(() => {
  // Bundled with the package's own esbuild settings so the code under test is the shipped
  // module, not a hand-transpiled stand-in.
  const compiled = join(BUILD_DIR, 'mcp-http-client.js');
  buildSync({
    entryPoints: [SRC],
    outfile: compiled,
    bundle: true,
    platform: 'node',
    target: 'node24',
    format: 'cjs',
    mainFields: ['module', 'main'],
    logLevel: 'silent',
  });
  ({
    McpHttpClient,
    McpHttpError,
    McpRpcError,
    McpProtocolError,
    McpTaskCancelledError,
    encodeHeaderValue,
    MCP_PROTOCOL_VERSION,
    TASKS_EXTENSION_ID,
  } = require(compiled));
});

afterAll(() => {
  rmSync(BUILD_DIR, { recursive: true, force: true });
});

const META = {
  protocolVersion: 'io.modelcontextprotocol/protocolVersion',
  clientInfo: 'io.modelcontextprotocol/clientInfo',
  clientCapabilities: 'io.modelcontextprotocol/clientCapabilities',
};

const CLIENT_INFO = { name: 'test-client', version: '9.9.9' };

/** The extension identifier as the specification spells it; the export must match it. */
const TASKS = 'io.modelcontextprotocol/tasks';

const DISCOVERY = {
  supportedVersions: ['2026-07-28', '2025-11-25'],
  capabilities: { tools: { listChanged: false }, extensions: { [TASKS]: {} } },
  serverInfo: { name: 'pierre-mcp-server', version: '1.2.3' },
  instructions: 'Fitness data for the athlete.',
  ttlMs: 3600000,
  cacheScope: 'public',
};


/** The endpoint every test starts from: discovery served, tools listed, hello answered. */
function pierreLike(overrides = {}) {
  return (rpc) => {
    if (overrides[rpc.method]) {
      return overrides[rpc.method](rpc);
    }
    switch (rpc.method) {
      case 'server/discover':
        return { json: complete(rpc.id, DISCOVERY) };
      case 'tools/list':
        return { json: complete(rpc.id, { tools: [{ name: 'hello', inputSchema: { type: 'object' } }], ttlMs: 60000, cacheScope: 'private' }) };
      case 'tools/call':
        return { json: complete(rpc.id, { content: [{ type: 'text', text: `hello ${rpc.params.arguments.who}` }], isError: false }) };
      default:
        return { status: 404, json: { jsonrpc: '2.0', id: rpc.id, error: { code: -32601, message: `Method not found: ${rpc.method}` } } };
    }
  };
}

function makeClient(url, overrides = {}) {
  return new McpHttpClient({ url, clientInfo: CLIENT_INFO, ...overrides });
}

describe('every request is self-describing', () => {
  let endpoint;
  afterEach(async () => {
    if (endpoint) {
      await endpoint.close();
    }
  });

  test('carries the protocol fields in _meta and the routing headers the binding requires', async () => {
    endpoint = await startEndpoint(pierreLike());
    const client = makeClient(endpoint.url, { bearer: async () => 'jwt-abc' });

    await client.discover();
    await client.callTool({ name: 'hello', arguments: { who: 'world' } });

    const [discover, call] = endpoint.seen;
    for (const request of [discover, call]) {
      expect(request.rpc.params._meta[META.protocolVersion]).toBe(MCP_PROTOCOL_VERSION);
      expect(request.rpc.params._meta[META.clientInfo]).toEqual(CLIENT_INFO);
      expect(request.rpc.params._meta[META.clientCapabilities]).toBeDefined();
      expect(request.headers['mcp-protocol-version']).toBe(MCP_PROTOCOL_VERSION);
      expect(request.headers['content-type']).toBe('application/json');
      expect(request.headers.accept).toBe('application/json, text/event-stream');
      expect(request.headers.authorization).toBe('Bearer jwt-abc');
    }
    expect(discover.headers['mcp-method']).toBe('server/discover');
    expect(discover.headers['mcp-name']).toBeUndefined();
    expect(call.headers['mcp-method']).toBe('tools/call');
    expect(call.headers['mcp-name']).toBe('hello');
    // Request ids are distinct: a retry is a new request, never a replay.
    expect(discover.rpc.id).not.toBe(call.rpc.id);
  });

  test('exports the protocol constants as the specification spells them', () => {
    expect(MCP_PROTOCOL_VERSION).toBe('2026-07-28');
    expect(TASKS_EXTENSION_ID).toBe(TASKS);
  });

  test('never sends initialize or ping - the modern era has neither', async () => {
    endpoint = await startEndpoint(pierreLike());
    const client = makeClient(endpoint.url);
    await client.discover();
    await client.listTools();
    expect(endpoint.seen.map((r) => r.rpc.method)).toEqual(['server/discover', 'tools/list']);
  });

  test('declares the Tasks extension only once discovery showed the server serves it', async () => {
    endpoint = await startEndpoint(pierreLike());
    const client = makeClient(endpoint.url);

    await client.callTool({ name: 'hello', arguments: { who: 'early' } });
    expect(endpoint.seen[0].rpc.params._meta[META.clientCapabilities]).toEqual({});
    expect(client.serverSupportsTasks()).toBe(false);

    await client.discover();
    expect(client.serverSupportsTasks()).toBe(true);
    expect(client.serverInfo).toEqual(DISCOVERY.serverInfo);
    expect(client.instructions).toBe(DISCOVERY.instructions);

    await client.callTool({ name: 'hello', arguments: { who: 'late' } });
    expect(endpoint.seen[2].rpc.params._meta[META.clientCapabilities]).toEqual({
      extensions: { [TASKS]: {} },
    });
  });

  test('a server without the extension is never told the client wants tasks', async () => {
    const plain = { ...DISCOVERY, capabilities: { tools: {} } };
    endpoint = await startEndpoint(pierreLike({ 'server/discover': (rpc) => ({ json: complete(rpc.id, plain) }) }));
    const client = makeClient(endpoint.url);
    await client.discover();
    expect(client.serverSupportsTasks()).toBe(false);
    await client.callTool({ name: 'hello', arguments: { who: 'x' } });
    expect(endpoint.seen[1].rpc.params._meta[META.clientCapabilities]).toEqual({});
  });

  test('omits the bearer when there is no session', async () => {
    endpoint = await startEndpoint(pierreLike());
    const client = makeClient(endpoint.url, { bearer: async () => undefined });
    await client.discover();
    expect(endpoint.seen[0].headers.authorization).toBeUndefined();
  });

  test('mirrors the resource uri and the task id into Mcp-Name, base64-encoding what HTTP cannot carry', async () => {
    endpoint = await startEndpoint(
      pierreLike({
        'resources/read': (rpc) => ({ json: complete(rpc.id, { contents: [] }) }),
        'tasks/get': (rpc) => ({
          json: complete(rpc.id, { taskId: rpc.params.taskId, status: 'working', createdAt: 't', lastUpdatedAt: 't', ttlMs: null }),
        }),
      }),
    );
    const client = makeClient(endpoint.url);

    await client.request('resources/read', { uri: 'dravr://athlete/profile' });
    await client.getTask('task-42');
    await client.request('resources/read', { uri: 'dravr://athlète/été' });

    expect(endpoint.seen[0].headers['mcp-name']).toBe('dravr://athlete/profile');
    expect(endpoint.seen[1].headers['mcp-name']).toBe('task-42');
    const encoded = endpoint.seen[2].headers['mcp-name'];
    expect(encoded).toMatch(/^=\?base64\?.+\?=$/);
    const decoded = Buffer.from(encoded.slice('=?base64?'.length, -'?='.length), 'base64').toString('utf8');
    expect(decoded).toBe('dravr://athlète/été');
  });
});

describe('encodeHeaderValue', () => {
  test('passes plain ASCII through and encodes everything else', () => {
    expect(encodeHeaderValue('get_activities')).toBe('get_activities');
    expect(encodeHeaderValue('file:///projects/app/config.json')).toBe('file:///projects/app/config.json');
    expect(encodeHeaderValue('Hello, 世界')).toBe('=?base64?SGVsbG8sIOS4lueVjA==?=');
    expect(encodeHeaderValue(' padded ')).toBe('=?base64?IHBhZGRlZCA=?=');
    expect(encodeHeaderValue('line1\nline2')).toBe('=?base64?bGluZTEKbGluZTI=?=');
    // A value that already looks like the sentinel is encoded so it cannot be misread.
    expect(encodeHeaderValue('=?base64?literal?=')).toBe('=?base64?PT9iYXNlNjQ/bGl0ZXJhbD89?=');
  });
});

describe('reading replies', () => {
  let endpoint;
  afterEach(async () => {
    if (endpoint) {
      await endpoint.close();
    }
  });

  test('strips resultType from a complete result and follows tools/list pagination', async () => {
    let page = 0;
    endpoint = await startEndpoint(
      pierreLike({
        'tools/list': (rpc) => {
          page += 1;
          if (!rpc.params.cursor) {
            return { json: complete(rpc.id, { tools: [{ name: 'a' }], nextCursor: 'page-2', ttlMs: 5000, cacheScope: 'private' }) };
          }
          expect(rpc.params.cursor).toBe('page-2');
          return { json: complete(rpc.id, { tools: [{ name: 'b' }], ttlMs: 5000, cacheScope: 'private' }) };
        },
      }),
    );
    const client = makeClient(endpoint.url);

    const listing = await client.listTools();
    expect(page).toBe(2);
    expect(listing.tools.map((t) => t.name)).toEqual(['a', 'b']);
    expect(listing.nextCursor).toBeUndefined();
    expect(listing.resultType).toBeUndefined();
    expect(listing.ttlMs).toBe(5000);
    expect(listing.cacheScope).toBe('private');

    const result = await client.callTool({ name: 'hello', arguments: { who: 'x' } });
    expect(result).toEqual({ content: [{ type: 'text', text: 'hello x' }], isError: false });
  });

  test('reads an SSE reply, delivering the request-scoped progress before the response', async () => {
    endpoint = await startEndpoint(
      pierreLike({
        'tools/call': (rpc) => ({
          sse: [
            { jsonrpc: '2.0', method: 'notifications/progress', params: { progressToken: rpc.params._meta.progressToken, progress: 1, total: 2, message: 'halfway' } },
            { jsonrpc: '2.0', method: 'notifications/progress', params: { progressToken: 'someone-else', progress: 9 } },
            complete(rpc.id, { content: [{ type: 'text', text: 'streamed' }] }),
          ],
        }),
      }),
    );
    const client = makeClient(endpoint.url);
    const progress = [];

    const result = await client.callTool(
      { name: 'hello', arguments: {} },
      { onProgress: (p) => progress.push(p) },
    );

    expect(result.content[0].text).toBe('streamed');
    // Only progress against this call's own token reaches the caller.
    expect(progress).toEqual([
      expect.objectContaining({ progress: 1, total: 2, message: 'halfway' }),
    ]);
    // The token went out with the request, so the server had something to report against.
    expect(endpoint.seen[0].rpc.params._meta.progressToken).toBe(progress[0].progressToken);
  });

  test('a JSON-RPC error in a 200 reply is raised with its code and data', async () => {
    endpoint = await startEndpoint(
      pierreLike({
        'tools/call': (rpc) => ({ json: { jsonrpc: '2.0', id: rpc.id, error: { code: -32602, message: 'Unknown tool: nope', data: { tool: 'nope' } } } }),
      }),
    );
    const client = makeClient(endpoint.url);
    await expect(client.callTool({ name: 'nope' })).rejects.toMatchObject({
      name: 'McpRpcError',
      code: -32602,
      message: 'Unknown tool: nope',
      data: { tool: 'nope' },
    });
  });

  test('an unsupported protocol version names the versions the server does speak', async () => {
    endpoint = await startEndpoint(() => ({
      status: 400,
      json: { jsonrpc: '2.0', id: 1, error: { code: -32022, message: 'Unsupported protocol version', data: { supported: ['2025-11-25'], requested: '2026-07-28' } } },
    }));
    const client = makeClient(endpoint.url);
    let caught;
    try {
      await client.discover();
    } catch (error) {
      caught = error;
    }
    expect(caught).toBeInstanceOf(McpRpcError);
    expect(caught.code).toBe(-32022);
    expect(caught.data.supported).toEqual(['2025-11-25']);
  });

  test('a 401 surfaces the RFC 9728 challenge and the JSON-RPC body the server sent', async () => {
    endpoint = await startEndpoint(() => ({
      status: 401,
      headers: { 'WWW-Authenticate': 'Bearer resource_metadata="https://api.example/.well-known/oauth-protected-resource"' },
      json: { jsonrpc: '2.0', id: null, error: { code: -32001, message: 'Unauthorized' } },
    }));
    const client = makeClient(endpoint.url);
    let caught;
    try {
      await client.listTools();
    } catch (error) {
      caught = error;
    }
    expect(caught).toBeInstanceOf(McpHttpError);
    expect(caught.status).toBe(401);
    expect(caught.wwwAuthenticate).toContain('resource_metadata=');
    expect(caught.rpc).toEqual({ code: -32001, message: 'Unauthorized' });
  });

  test('a 404 with method-not-found is the JSON-RPC error, not a bare transport failure', async () => {
    endpoint = await startEndpoint(pierreLike());
    const client = makeClient(endpoint.url);
    await expect(client.request('logging/setLevel', { level: 'debug' })).rejects.toMatchObject({
      name: 'McpRpcError',
      code: -32601,
    });
  });

  test('a result of a shape a plain request cannot carry is a protocol error', async () => {
    endpoint = await startEndpoint(
      pierreLike({
        'tools/list': (rpc) => ({ json: { jsonrpc: '2.0', id: rpc.id, result: { resultType: 'task', taskId: 't1' } } }),
        'tools/call': (rpc) => ({ json: { jsonrpc: '2.0', id: rpc.id, result: { resultType: 'someday' } } }),
      }),
    );
    const client = makeClient(endpoint.url);
    await expect(client.listTools()).rejects.toBeInstanceOf(McpProtocolError);
    await expect(client.callTool({ name: 'hello' })).rejects.toThrow(/unknown resultType "someday"/);
  });

  test('close() abandons what is in flight', async () => {
    endpoint = await startEndpoint(() => ({ hold: true }));
    // The endpoint never answers, so only close() can settle the request.
    const client = makeClient(endpoint.url);
    const outcome = client.discover().then(
      () => 'resolved',
      (error) => error,
    );
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(endpoint.seen).toHaveLength(1);

    client.close();

    const settled = await outcome;
    expect(settled).toBeInstanceOf(Error);
    expect(settled.message).toMatch(/closed/i);
    await expect(client.discover()).rejects.toThrow(/closed/);
  });
});

describe('the input_required round trip', () => {
  let endpoint;
  afterEach(async () => {
    if (endpoint) {
      await endpoint.close();
    }
  });

  test('retries with the echoed requestState when no input is actually requested', async () => {
    let calls = 0;
    endpoint = await startEndpoint(
      pierreLike({
        'tools/call': (rpc) => {
          calls += 1;
          if (calls === 1) {
            return { json: { jsonrpc: '2.0', id: rpc.id, result: { resultType: 'input_required', requestState: 'opaque-state' } } };
          }
          expect(rpc.params.requestState).toBe('opaque-state');
          return { json: complete(rpc.id, { content: [{ type: 'text', text: 'done after retry' }] }) };
        },
      }),
    );
    const client = makeClient(endpoint.url);
    const result = await client.callTool({ name: 'hello', arguments: { who: 'x' } });
    expect(calls).toBe(2);
    expect(result.content[0].text).toBe('done after retry');
    // Each attempt is its own request.
    expect(endpoint.seen[0].rpc.id).not.toBe(endpoint.seen[1].rpc.id);
  });

  test('refuses an input request this client declares no capability for', async () => {
    endpoint = await startEndpoint(
      pierreLike({
        'tools/call': (rpc) => ({
          json: { jsonrpc: '2.0', id: rpc.id, result: { resultType: 'input_required', inputRequests: { login: { method: 'elicitation/create', params: {} } } } },
        }),
      }),
    );
    const client = makeClient(endpoint.url);
    await expect(client.callTool({ name: 'hello' })).rejects.toThrow(/does not provide \(login\)/);
    expect(endpoint.seen).toHaveLength(1);
  });
});

describe('Tasks: a call answered with a handle is polled to its result', () => {
  let endpoint;
  afterEach(async () => {
    if (endpoint) {
      await endpoint.close();
    }
  });

  const handle = (rpc, extra = {}) => ({
    json: {
      jsonrpc: '2.0',
      id: rpc.id,
      result: { resultType: 'task', taskId: 'task-1', status: 'working', createdAt: 't0', lastUpdatedAt: 't0', ttlMs: 60000, pollIntervalMs: 10, ...extra },
    },
  });
  const state = (rpc, status, extra = {}) => ({
    json: complete(rpc.id, { taskId: 'task-1', status, createdAt: 't0', lastUpdatedAt: 't1', ttlMs: 60000, pollIntervalMs: 10, ...extra }),
  });

  test('polls tasks/get at the server cadence and returns the completed result', async () => {
    let polls = 0;
    endpoint = await startEndpoint(
      pierreLike({
        'tools/call': (rpc) => handle(rpc),
        'tasks/get': (rpc) => {
          polls += 1;
          expect(rpc.params.taskId).toBe('task-1');
          return polls < 3
            ? state(rpc, 'working', { statusMessage: `step ${polls}` })
            : state(rpc, 'completed', { result: { resultType: 'complete', content: [{ type: 'text', text: '12 activities' }], isError: false } });
        },
      }),
    );
    const client = makeClient(endpoint.url);
    await client.discover();
    const tasks = [];
    const updates = [];

    const result = await client.callTool(
      { name: 'get_activities', arguments: { limit: 500 } },
      { onTask: (t) => tasks.push(t), onTaskUpdate: (s) => updates.push(s.status) },
    );

    expect(result).toEqual({ content: [{ type: 'text', text: '12 activities' }], isError: false });
    expect(tasks).toEqual([expect.objectContaining({ taskId: 'task-1', status: 'working', pollIntervalMs: 10 })]);
    expect(updates).toEqual(['working', 'working', 'completed']);
    expect(endpoint.seen.map((r) => r.rpc.method)).toEqual(['server/discover', 'tools/call', 'tasks/get', 'tasks/get', 'tasks/get']);
    // The polls carry the extension declaration too - the server gates tasks/* on it.
    expect(endpoint.seen[2].rpc.params._meta[META.clientCapabilities].extensions[TASKS]).toEqual({});
  });

  test('a failed task raises the JSON-RPC error it recorded', async () => {
    endpoint = await startEndpoint(
      pierreLike({
        'tools/call': (rpc) => handle(rpc),
        'tasks/get': (rpc) => state(rpc, 'failed', { error: { code: -32603, message: 'provider timed out', data: { provider: 'strava' } } }),
      }),
    );
    const client = makeClient(endpoint.url);
    await client.discover();
    await expect(client.callTool({ name: 'get_activities' })).rejects.toMatchObject({
      name: 'McpRpcError',
      code: -32603,
      message: 'provider timed out',
      data: { provider: 'strava' },
    });
  });

  test('a cancelled task is reported as cancelled, with the server message', async () => {
    endpoint = await startEndpoint(
      pierreLike({
        'tools/call': (rpc) => handle(rpc),
        'tasks/get': (rpc) => state(rpc, 'cancelled', { statusMessage: 'operator stopped it' }),
      }),
    );
    const client = makeClient(endpoint.url);
    await client.discover();
    let caught;
    try {
      await client.callTool({ name: 'get_activities' });
    } catch (error) {
      caught = error;
    }
    expect(caught).toBeInstanceOf(McpTaskCancelledError);
    expect(caught.message).toMatch(/operator stopped it/);
    expect(caught.task.taskId).toBe('task-1');
  });

  test('the caller aborting while polling cancels the task server-side', async () => {
    const methods = [];
    endpoint = await startEndpoint(
      pierreLike({
        'tools/call': (rpc) => handle(rpc, { pollIntervalMs: 20 }),
        'tasks/get': (rpc) => {
          methods.push('tasks/get');
          return state(rpc, 'working');
        },
        'tasks/cancel': (rpc) => {
          methods.push('tasks/cancel');
          return { json: complete(rpc.id, {}) };
        },
      }),
    );
    const client = makeClient(endpoint.url);
    await client.discover();
    const controller = new AbortController();

    const pending = client.callTool({ name: 'get_activities' }, { signal: controller.signal });
    await new Promise((resolve) => setTimeout(resolve, 55));
    controller.abort();
    await expect(pending).rejects.toMatchObject({ name: 'AbortError' });
    // Give the fire-and-forget cancel a turn to land.
    await new Promise((resolve) => setTimeout(resolve, 30));

    expect(methods).toContain('tasks/get');
    expect(methods[methods.length - 1]).toBe('tasks/cancel');
    const cancel = endpoint.seen[endpoint.seen.length - 1];
    expect(cancel.rpc.params.taskId).toBe('task-1');
    expect(cancel.headers['mcp-name']).toBe('task-1');
  });

  test('a task asking for input is cancelled and refused, never left waiting', async () => {
    const methods = [];
    endpoint = await startEndpoint(
      pierreLike({
        'tools/call': (rpc) => handle(rpc),
        'tasks/get': (rpc) => state(rpc, 'input_required', { inputRequests: { confirm: { method: 'elicitation/create', params: {} } } }),
        'tasks/cancel': (rpc) => {
          methods.push('tasks/cancel');
          return { json: complete(rpc.id, {}) };
        },
      }),
    );
    const client = makeClient(endpoint.url);
    await client.discover();
    await expect(client.callTool({ name: 'get_activities' })).rejects.toThrow(/does not provide \(confirm\)/);
    expect(methods).toEqual(['tasks/cancel']);
  });
});
