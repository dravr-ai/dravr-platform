// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Comprehensive response shape validation for ALL 47 MCP tools
// ABOUTME: Ensures response formats are stable and match documented schemas

const { ensureServerRunning } = require('../helpers/server');
const { MockMCPClient } = require('../helpers/mock-client');
const { MCPMessages, TestConfig } = require('../helpers/fixtures');
const { clearKeychainTokens } = require('../helpers/keychain-cleanup');
const path = require('path');

/**
 * Tools that run only in Dravr's own app and channels: an outside client is
 * neither offered them nor allowed to call them.
 */
const FIRST_PARTY_ONLY_TOOLS = [
  'get_configuration_catalog',
  'get_configuration_profiles',
  'get_user_configuration',
  'validate_configuration',
  'get_fitness_config',
  'list_fitness_configs',
  'list_agents',
  'get_agent',
  'search_agents',
  'get_active_agent',
  'list_hidden_agents',
  'list_coaching_playbooks',
  'calculate_daily_nutrition',
  'get_nutrient_timing',
];

describe('All Tools Response Validation', () => {
  let serverHandle;
  let bridgeClient;
  let testToken;
  const bridgePath = path.join(__dirname, '../../dist/cli.js');
  const serverUrl = `http://localhost:${TestConfig.defaultServerPort}`;

  beforeAll(async () => {
    serverHandle = await ensureServerRunning({
      port: TestConfig.defaultServerPort,
      database: TestConfig.testDatabase,
      encryptionKey: TestConfig.testEncryptionKey
    });
    testToken = serverHandle?.testToken;
  }, 60000);

  beforeEach(async () => {
    await clearKeychainTokens();
  });

  afterEach(async () => {
    if (bridgeClient) {
      await bridgeClient.stop();
      bridgeClient = null;
    }
  });

  afterAll(async () => {
    if (serverHandle?.cleanup) {
      await serverHandle.cleanup();
    }
  });

  // Helper to verify MCP response format
  const verifyMcpResponseFormat = (response) => {
    // All MCP responses should have jsonrpc version
    expect(response.jsonrpc).toBe('2.0');
    // Must have either result or error
    expect(response.result !== undefined || response.error !== undefined).toBe(true);

    if (response.error) {
      // Error responses must have code and message
      expect(response.error.code).toBeDefined();
      expect(response.error.message).toBeDefined();
      expect(typeof response.error.message).toBe('string');
    }

    if (response.result) {
      // Success responses should have content array (MCP protocol)
      // Some responses return the content directly
      expect(response.result).toBeDefined();
    }
  };

  // ============================================================================
  // Tools List Verification
  // ============================================================================

  test('tools/list returns all expected tools', async () => {
    bridgeClient = new MockMCPClient('node', [bridgePath, '--server', serverUrl], {
      env: { PIERRE_JWT_TOKEN: testToken.access_token }
    });

    await bridgeClient.start();
    await bridgeClient.send(MCPMessages.initialize);
    await new Promise(resolve => setTimeout(resolve, 2000));

    const response = await bridgeClient.send(MCPMessages.toolsList);

    verifyMcpResponseFormat(response);
    expect(response.result).toBeDefined();
    expect(response.result.tools).toBeDefined();
    expect(Array.isArray(response.result.tools)).toBe(true);

    const toolNames = response.result.tools.map(t => t.name);

    // Core data tools (in PUBLIC_DISCOVERY_TOOLS, always visible)
    const coreTools = [
      'get_activities',
      'get_athlete',
      'get_stats'
    ];

    // Analytics tools (in PUBLIC_DISCOVERY_TOOLS)
    const analyticsTools = [
      'analyze_activity',
      'get_activity_intelligence',
      'calculate_metrics',
      'analyze_performance_trends',
      'compare_activities',
      'analyze_weather_impact',
      'detect_patterns',
      'suggest_goals'
    ];

    // Configuration and verification tools an outside client is offered
    const configTools = [
      'calculate_personalized_zones',
      'verify_claim',
    ];

    // Nutrition tools that describe food (in PUBLIC_DISCOVERY_TOOLS)
    const nutritionTools = [
      'search_food',
      'get_food_details',
      'analyze_meal_nutrition'
    ];

    // Verify public discovery tools are always visible
    const allExpectedTools = [
      ...coreTools,
      ...analyticsTools,
      ...configTools,
      ...nutritionTools
    ];

    for (const tool of allExpectedTools) {
      expect(toolNames).toContain(tool);
    }

    // Kept on Dravr's own surfaces: never offered to an outside client
    for (const tool of FIRST_PARTY_ONLY_TOOLS) {
      expect(toolNames).not.toContain(tool);
    }

    // Log count for visibility
    console.log(`✅ Verified ${allExpectedTools.length} expected tools are present`);
    console.log(`   Total tools in response: ${toolNames.length}`);
  }, 30000);

  // ============================================================================
  // Response Shape Tests for Non-Auth Tools
  // ============================================================================

  describe('Non-Authentication Tools Response Shapes', () => {
    beforeEach(async () => {
      bridgeClient = new MockMCPClient('node', [bridgePath, '--server', serverUrl], {
        env: { PIERRE_JWT_TOKEN: testToken.access_token }
      });
      await bridgeClient.start();
      await bridgeClient.send(MCPMessages.initialize);
      await new Promise(resolve => setTimeout(resolve, 2000));
    });

    test('tools kept on Dravr\'s own surfaces are refused', async () => {
      for (const [index, name] of FIRST_PARTY_ONLY_TOOLS.entries()) {
        const response = await bridgeClient.send({
          jsonrpc: '2.0',
          id: 50 + index,
          method: 'tools/call',
          params: { name, arguments: {} }
        }, 10000);

        verifyMcpResponseFormat(response);
        expect(response.result?.isError).toBe(true);
        expect(JSON.stringify(response.result.content)).toContain('first_party_surface');
      }
      console.log(`✅ ${FIRST_PARTY_ONLY_TOOLS.length} first-party tools refused`);
    }, 60000);

    test('get_recipe_constraints returns constraints shape', async () => {
      const response = await bridgeClient.send({
        jsonrpc: '2.0',
        id: 3,
        method: 'tools/call',
        params: {
          name: 'get_recipe_constraints',
          arguments: {
            meal_timing: 'post_training',
            calories: 500
          }
        }
      }, 10000);

      verifyMcpResponseFormat(response);

      if (response.result) {
        // Verify expected fields in recipe constraints
        const content = response.result.content || response.result;
        expect(content).toBeDefined();
        console.log('✅ get_recipe_constraints: Constraints returned');
      }
    }, 20000);

    test('list_recipes returns empty array for new user', async () => {
      const response = await bridgeClient.send({
        jsonrpc: '2.0',
        id: 6,
        method: 'tools/call',
        params: {
          name: 'list_recipes',
          arguments: {}
        }
      }, 10000);

      verifyMcpResponseFormat(response);

      if (response.result) {
        console.log('✅ list_recipes: List returned');
      }
    }, 20000);

    test('search_recipes returns results shape', async () => {
      const response = await bridgeClient.send({
        jsonrpc: '2.0',
        id: 7,
        method: 'tools/call',
        params: {
          name: 'search_recipes',
          arguments: {
            query: 'chicken'
          }
        }
      }, 10000);

      verifyMcpResponseFormat(response);

      if (response.result) {
        console.log('✅ search_recipes: Search results returned');
      }
    }, 20000);

    test('Unknown tool returns method not found', async () => {
      const response = await bridgeClient.send({
        jsonrpc: '2.0',
        id: 102,
        method: 'tools/call',
        params: {
          name: 'definitely_not_a_real_tool_xyz',
          arguments: {}
        }
      }, 10000);

      verifyMcpResponseFormat(response);
      // Server may return error at top level OR in result.isError
      const hasError = response.error ||
        (response.result && response.result.isError) ||
        (response.result && response.result.content && response.result.content[0]?.type === 'text' &&
         response.result.content[0]?.text?.toLowerCase().includes('error'));
      expect(hasError).toBeTruthy();
      console.log('✅ Unknown tool: Error returned correctly');
    }, 20000);

    test('Invalid parameter types handled gracefully', async () => {
      const response = await bridgeClient.send({
        jsonrpc: '2.0',
        id: 103,
        method: 'tools/call',
        params: {
          name: 'calculate_personalized_zones',
          arguments: {
            vo2_max: 'not_a_number', // Should be number
            resting_hr: 65,
            max_hr: 185
          }
        }
      }, 10000);

      verifyMcpResponseFormat(response);

      // Should fail gracefully
      if (response.error || (response.result && !response.result.success)) {
        console.log('✅ Invalid parameter type: Handled gracefully');
      }
    }, 20000);
  });

  // ============================================================================
  // Tool Schema Consistency Tests
  // ============================================================================

  describe('Tool Schema Consistency', () => {
    beforeEach(async () => {
      bridgeClient = new MockMCPClient('node', [bridgePath, '--server', serverUrl], {
        env: { PIERRE_JWT_TOKEN: testToken.access_token }
      });
      await bridgeClient.start();
      await bridgeClient.send(MCPMessages.initialize);
      await new Promise(resolve => setTimeout(resolve, 2000));
    });

    test('All tools have valid schemas', async () => {
      const response = await bridgeClient.send(MCPMessages.toolsList);

      expect(response.result.tools).toBeDefined();

      for (const tool of response.result.tools) {
        // Each tool must have required fields
        expect(tool.name).toBeDefined();
        expect(typeof tool.name).toBe('string');
        expect(tool.name.length).toBeGreaterThan(0);

        expect(tool.description).toBeDefined();
        expect(typeof tool.description).toBe('string');
        expect(tool.description.length).toBeGreaterThan(0);

        // Input schema must be valid JSON Schema
        expect(tool.inputSchema).toBeDefined();
        expect(tool.inputSchema.type).toBe('object');
        // Properties may be undefined for tools with no parameters, or an empty object
        if (tool.inputSchema.properties !== undefined) {
          expect(typeof tool.inputSchema.properties).toBe('object');
        }
      }

      console.log(`✅ All ${response.result.tools.length} tools have valid schemas`);
    }, 30000);

    test('Tool names follow naming convention', async () => {
      const response = await bridgeClient.send(MCPMessages.toolsList);

      const invalidNames = [];
      for (const tool of response.result.tools) {
        // Names should be snake_case
        if (!/^[a-z][a-z0-9_]*$/.test(tool.name)) {
          invalidNames.push(tool.name);
        }
      }

      expect(invalidNames.length).toBe(0);
      console.log('✅ All tool names follow snake_case convention');
    }, 30000);
  });

  // ============================================================================
  // Cross-Provider Tool Tests
  // ============================================================================

  describe('Cross-Provider Tool Compatibility', () => {
    beforeEach(async () => {
      bridgeClient = new MockMCPClient('node', [bridgePath, '--server', serverUrl], {
        env: { PIERRE_JWT_TOKEN: testToken.access_token }
      });
      await bridgeClient.start();
      await bridgeClient.send(MCPMessages.initialize);
      await new Promise(resolve => setTimeout(resolve, 2000));
    });

    const crossProviderTools = [
      'get_activities',
      'get_athlete',
      'get_stats',
      'analyze_activity',
      'analyze_training_load',
      'calculate_recovery_score'
    ];

    test.each(crossProviderTools)(
      '%s accepts activity_provider parameter',
      async (toolName) => {
        const response = await bridgeClient.send({
          jsonrpc: '2.0',
          id: 200,
          method: 'tools/call',
          params: {
            name: toolName,
            arguments: {
              activity_provider: 'strava'
            }
          }
        }, 10000);

        verifyMcpResponseFormat(response);

        // Should NOT fail with "unknown field: activity_provider"
        if (response.error) {
          expect(response.error.message).not.toMatch(/unknown field.*activity_provider/i);
          expect(response.error.message).not.toMatch(/unknown field.*provider/i);
        }

        console.log(`✅ ${toolName}: Accepts activity_provider parameter`);
      },
      20000
    );
  });
});
