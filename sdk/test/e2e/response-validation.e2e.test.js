// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E tests that the server's advertised outputSchemas compile and real tool results conform to them
// ABOUTME: Lists tools from a running Pierre server, registers the schemas, calls read-only tools, validates structuredContent

const { ensureServerRunning } = require('../helpers/server');
const { TestConfig } = require('../helpers/fixtures');
const {
  configureValidator,
  hasResponseSchema,
  registerToolOutputSchemas,
  validateMcpToolResponse,
} = require('../../dist/index.js');

/**
 * Read-only tools that answer a fresh test user without a connected provider.
 * Between them they cover a plain object answer, the `Formatted<T>` anyOf
 * answer, and nested `$defs` types.
 */
const READ_ONLY_TOOLS = [
  'get_connection_status',
  'get_configuration_catalog',
  'get_configuration_profiles',
  'get_user_configuration',
  'get_fitness_config',
  'list_fitness_configs',
  'get_data_freshness',
  'list_data_sources',
  'get_training_history',
  'list_coaching_playbooks',
  'list_recipes',
  'list_stretching_exercises',
  'list_yoga_poses',
  'list_workout_templates',
  'calculate_personalized_zones',
];

describe('E2E: tool results conform to the advertised outputSchema', () => {
  let token;
  let tools;
  let requestId = 1;
  const port = TestConfig.defaultServerPort;

  async function rpc(method, params) {
    const response = await fetch(`http://localhost:${port}/mcp`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${token}` },
      body: JSON.stringify({ jsonrpc: '2.0', id: requestId++, method, params }),
    });
    return response.json();
  }

  beforeAll(async () => {
    const serverHandle = await ensureServerRunning({
      port,
      database: TestConfig.testDatabase,
      encryptionKey: TestConfig.testEncryptionKey,
    });
    token = serverHandle.testToken.access_token;
    tools = (await rpc('tools/list', {})).result.tools;
  }, 90000);

  beforeEach(() => {
    configureValidator({ enabled: true, strict: false, logger: () => {} });
  });

  afterAll(() => {
    registerToolOutputSchemas([]);
    configureValidator({
      enabled: process.env.NODE_ENV !== 'production',
      strict: false,
      logRawData: false,
      logger: undefined,
    });
  });

  test('every advertised outputSchema has an object root and compiles', () => {
    const declared = tools.filter((tool) => tool.outputSchema);
    // A listing with no schemas would make every check below vacuous.
    expect(declared.length).toBeGreaterThan(100);

    const notObjectRooted = declared
      .filter((tool) => tool.outputSchema.type !== 'object')
      .map((tool) => tool.name);
    expect(notObjectRooted).toEqual([]);

    const messages = [];
    configureValidator({ logger: (message) => messages.push(message) });
    registerToolOutputSchemas(tools);

    expect(messages).toEqual([]);
    expect(declared.filter((tool) => !hasResponseSchema(tool.name)).map((tool) => tool.name)).toEqual([]);
  });

  test.each(READ_ONLY_TOOLS)(
    '%s answers structuredContent that validates against its outputSchema',
    async (name) => {
      registerToolOutputSchemas(tools);
      const response = await rpc('tools/call', { name, arguments: {} });
      const result = response.result;

      expect(result.isError).toBe(false);
      expect(result.structuredContent).toBeDefined();

      const validated = validateMcpToolResponse(name, result);
      expect(validated.errors).toBeUndefined();
      expect(validated.valid).toBe(true);
      expect(validated.data).toEqual(result.structuredContent);
    },
    30000,
  );
});
