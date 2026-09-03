// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Integration tests driving a real Dravr server through McpHttpClient, revision 2026-07-28
// ABOUTME: Asserts the wire shapes the bridge depends on — the roster, an in-band tool error, a 401 challenge

/**
 * These drive the client the SDK actually ships.
 *
 * They used to build `Client` + `StreamableHTTPClientTransport` from the
 * official MCP SDK — a transport `sdk/src` abandoned when the bridge moved to
 * revision 2026-07-28, and one the server serves only for its legacy era. So
 * the suite reported on a code path no Dravr user reaches, and most of its
 * assertions were `try { … } catch { expect(error).toBeDefined() }`, which
 * passes whatever happens.
 */

const {
  McpHttpClient,
  McpHttpError,
  MCP_PROTOCOL_VERSION,
  TOOL_NAMES,
} = require('../../dist/index.js');
const { ensureServerRunning } = require('../helpers/server');
const { TestConfig } = require('../helpers/fixtures');

describe('Dravr over Streamable HTTP', () => {
  let serverHandle;
  let testToken;
  const serverUrl = `http://localhost:${TestConfig.defaultServerPort}`;
  const mcpUrl = `${serverUrl}/mcp`;
  const open = [];

  /** A client authenticated as the suite's admin test user. */
  function client(bearer = () => testToken.access_token) {
    const c = new McpHttpClient({
      url: mcpUrl,
      clientInfo: { name: 'sdk-integration', version: '0.0.0' },
      bearer,
    });
    open.push(c);
    return c;
  }

  beforeAll(async () => {
    serverHandle = await ensureServerRunning({
      port: TestConfig.defaultServerPort,
      database: TestConfig.testDatabase,
      encryptionKey: TestConfig.testEncryptionKey,
    });
    testToken = serverHandle?.testToken;
  }, 60000);

  afterEach(() => {
    while (open.length) {
      open.pop().close();
    }
  });

  afterAll(async () => {
    if (serverHandle?.cleanup) {
      await serverHandle.cleanup();
    }
  });

  describe('discovery', () => {
    test('the server is up and names itself', async () => {
      const health = await fetch(`${serverUrl}/health`).then((r) => r.json());
      expect(health.status).toBe('ok');
      expect(health.service).toBe('pierre-mcp-server');
    });

    test('server/discover names the revision this client speaks and the Tasks extension', async () => {
      const discovery = await client().discover();

      expect(discovery.supportedVersions[0]).toBe(MCP_PROTOCOL_VERSION);
      expect(discovery.serverInfo?.name).toBe('pierre-mcp-server');
      // An array path, not a string: the extension's name contains dots, and
      // `toHaveProperty` splits a string path on them — it went looking for
      // `io` → `modelcontextprotocol/tasks` and reported the key as missing
      // while printing it in the received object.
      expect(discovery.capabilities.extensions).toHaveProperty([
        'io.modelcontextprotocol/tasks',
      ]);
    });
  });

  describe('tools/list', () => {
    /**
     * The generated roster is the expectation. The suite used to keep a jest
     * snapshot of the same names, and CI ran it with --updateSnapshot, so a
     * tool disappearing rewrote the expectation instead of failing.
     */
    test('returns exactly the generated roster', async () => {
      const { tools } = await client().listTools();
      const names = tools.map((t) => t.name).sort();

      expect(names).toEqual([...TOOL_NAMES]);
    });

    test('every tool carries a description and an object input schema', async () => {
      const { tools } = await client().listTools();

      for (const tool of tools) {
        expect(typeof tool.description).toBe('string');
        expect(tool.description.length).toBeGreaterThan(0);
        expect(tool.inputSchema.type).toBe('object');
        const required = tool.inputSchema.required ?? [];
        const properties = Object.keys(tool.inputSchema.properties ?? {});
        for (const field of required) {
          expect(properties).toContain(field);
        }
      }
    });
  });

  describe('tools/call', () => {
    /**
     * What `/mcp` answers, which is NOT what the registry tool answers.
     *
     * `tool_handlers.rs` intercepts `get_connection_status` before the registry
     * and shapes its own payload: an array of two hardcoded providers carrying
     * `connect_url` and `connect_instructions`, where the registry tool returns
     * an object keyed by provider carrying `status` and `backend`. This test
     * pins the transport's real answer rather than the one the rest of the
     * product sees — see carnet#233, which is the divergence itself. When that
     * is resolved this asserts the registry shape.
     */
    test('get_connection_status answers in band with a provider list', async () => {
      const result = await client().callTool({
        name: 'get_connection_status',
        arguments: {},
      });

      expect(result.isError).toBeFalsy();
      const providers = result.structuredContent?.providers ?? [];
      expect(Array.isArray(providers)).toBe(true);
      expect(providers.length).toBeGreaterThan(0);
      // A fresh test user has connected none of them, and every entry carries
      // the way in.
      for (const entry of providers) {
        expect(typeof entry.provider).toBe('string');
        expect(entry.connected).toBe(false);
        expect(typeof entry.connect_url).toBe('string');
      }
    });

    /**
     * A tool that needs data the athlete has not connected fails *in band* —
     * `isError` on a 200 — never as a JSON-RPC error. The bridge relies on that
     * distinction: a thrown error is a transport problem, this is an answer.
     */
    test('get_activities without a connected provider is an in-band refusal', async () => {
      const result = await client().callTool({
        name: 'get_activities',
        arguments: { provider: 'strava', limit: 10 },
      });

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain('No fitness provider connected');
    });

  });

  describe('failures', () => {
    /**
     * The one signal that means "this session is dead", and the only one the
     * bridge acts on. Its shape is load-bearing: the challenge is what tells a
     * host where to re-authenticate.
     */
    test('a call with no bearer is a 401 carrying the RFC 9728 challenge', async () => {
      const anonymous = client(() => undefined);

      await expect(anonymous.listTools()).rejects.toThrow(McpHttpError);
      const error = await anonymous.listTools().catch((e) => e);

      expect(error.status).toBe(401);
      expect(error.wwwAuthenticate).toMatch(
        /^Bearer resource_metadata="http.*\/\.well-known\/oauth-protected-resource"/,
      );
      expect(error.rpc?.code).toBe(-32001);
    });

    test('an unreachable endpoint rejects rather than hanging', async () => {
      const unreachable = new McpHttpClient({
        url: 'http://127.0.0.1:19999/mcp',
        clientInfo: { name: 'sdk-integration', version: '0.0.0' },
      });

      await expect(unreachable.listTools()).rejects.toThrow();
      unreachable.close();
    });
  });

  /**
   * Three clients against one server: the revision is stateless, so there is no
   * session to collide over and each gets the same roster.
   */
  test('concurrent clients each see the whole roster', async () => {
    const clients = [client(), client(), client()];

    const results = await Promise.all(clients.map((c) => c.listTools()));

    for (const { tools } of results) {
      expect(tools.map((t) => t.name).sort()).toEqual([...TOOL_NAMES]);
    }
  });
});
