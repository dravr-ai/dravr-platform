// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for response validation utilities and configuration
// ABOUTME: Tests validator configuration, stats tracking, and MCP tool response validation

const {
  configureValidator,
  getValidatorConfig,
  getValidationStats,
  resetValidationStats,
  validateMcpToolResponse,
  isValidResponse,
  validateWithStats,
  hasResponseSchema,
  validateToolResponse,
} = require('../../dist/index.js');

describe('Response Validator Configuration', () => {
  afterEach(() => {
    // Reset to defaults after each test
    configureValidator({
      enabled: process.env.NODE_ENV !== 'production',
      strict: false,
      logRawData: false,
      logger: undefined,
    });
  });

  test('should have sensible default configuration', () => {
    const config = getValidatorConfig();

    expect(config).toHaveProperty('enabled');
    expect(config).toHaveProperty('strict');
    expect(config).toHaveProperty('logRawData');
    expect(typeof config.enabled).toBe('boolean');
    expect(typeof config.strict).toBe('boolean');
    expect(config.logRawData).toBe(false);
  });

  test('should update configuration with partial overrides', () => {
    configureValidator({ strict: true });

    const config = getValidatorConfig();
    expect(config.strict).toBe(true);
  });

  test('should support enabling/disabling validation', () => {
    configureValidator({ enabled: false });
    expect(getValidatorConfig().enabled).toBe(false);

    configureValidator({ enabled: true });
    expect(getValidatorConfig().enabled).toBe(true);
  });

  test('should support custom logger function', () => {
    const messages = [];
    configureValidator({
      logger: (msg) => messages.push(msg),
    });

    const config = getValidatorConfig();
    expect(typeof config.logger).toBe('function');
  });

  test('should preserve existing settings when updating a single field', () => {
    configureValidator({ strict: true, logRawData: true });
    configureValidator({ strict: false });

    const config = getValidatorConfig();
    expect(config.strict).toBe(false);
    expect(config.logRawData).toBe(true);
  });

  test('config should return a copy (not a mutable reference)', () => {
    const config1 = getValidatorConfig();
    const config2 = getValidatorConfig();

    expect(config1).toEqual(config2);
    expect(config1).not.toBe(config2);
  });
});

describe('Validation Statistics', () => {
  beforeEach(() => {
    resetValidationStats();
    // Suppress log output during tests
    configureValidator({ enabled: true, strict: false, logger: () => {} });
  });

  afterEach(() => {
    configureValidator({
      enabled: process.env.NODE_ENV !== 'production',
      strict: false,
      logRawData: false,
      logger: undefined,
    });
  });

  test('should start with zero stats after reset', () => {
    const stats = getValidationStats();

    expect(stats.totalCalls).toBe(0);
    expect(stats.validResponses).toBe(0);
    expect(stats.invalidResponses).toBe(0);
    expect(stats.skippedResponses).toBe(0);
  });

  test('should track validation attempts via validateWithStats', () => {
    // validateWithStats expects MCP-wrapped result format
    const mcpResult = {
      content: [
        {
          type: 'text',
          text: JSON.stringify({ activities: [], provider: 'strava' }),
        },
      ],
    };

    validateWithStats('get_activities', mcpResult);

    const stats = getValidationStats();
    expect(stats.totalCalls).toBe(1);
  });

  test('should count valid responses', () => {
    const mcpResult = {
      content: [
        {
          type: 'text',
          text: JSON.stringify({
            activities: [{ id: '1', name: 'Run', type: 'Run', distance: 5000, duration: 1800 }],
            provider: 'strava',
            count: 1,
            has_more: false,
          }),
        },
      ],
    };

    validateWithStats('get_activities', mcpResult);

    const stats = getValidationStats();
    expect(stats.validResponses).toBeGreaterThan(0);
  });

  test('should count invalid responses', () => {
    const mcpResult = {
      content: [
        {
          type: 'text',
          text: JSON.stringify({ fitness_score: 200 }), // Invalid: > 100
        },
      ],
    };

    validateWithStats('calculate_fitness_score', mcpResult);

    const stats = getValidationStats();
    expect(stats.invalidResponses).toBeGreaterThan(0);
    expect(stats.errorsByTool).toHaveProperty('calculate_fitness_score');
  });

  test('should reset stats correctly', () => {
    const mcpResult = {
      content: [
        { type: 'text', text: JSON.stringify({ activities: [], provider: 'strava' }) },
      ],
    };

    validateWithStats('get_activities', mcpResult);

    let stats = getValidationStats();
    expect(stats.totalCalls).toBeGreaterThan(0);

    resetValidationStats();
    stats = getValidationStats();
    expect(stats.totalCalls).toBe(0);
    expect(stats.validResponses).toBe(0);
    expect(stats.invalidResponses).toBe(0);
  });
});

describe('isValidResponse Type Guard', () => {
  beforeEach(() => {
    configureValidator({ enabled: true, strict: false, logger: () => {} });
  });

  afterEach(() => {
    configureValidator({
      enabled: process.env.NODE_ENV !== 'production',
      strict: false,
      logRawData: false,
      logger: undefined,
    });
  });

  test('should return true for valid validated result', () => {
    const mcpResult = {
      content: [
        {
          type: 'text',
          text: JSON.stringify({
            provider: 'strava',
            status: 'connected',
            connected: true,
          }),
        },
      ],
    };

    const validated = validateMcpToolResponse('get_connection_status', mcpResult);
    expect(isValidResponse(validated)).toBe(true);
  });

  test('should return false for invalid validated result', () => {
    const mcpResult = {
      content: [
        {
          type: 'text',
          text: JSON.stringify({ fitness_score: -50 }),
        },
      ],
    };

    const validated = validateMcpToolResponse('calculate_fitness_score', mcpResult);
    expect(isValidResponse(validated)).toBe(false);
  });
});

describe('validateMcpToolResponse', () => {
  beforeEach(() => {
    configureValidator({ enabled: true, strict: false, logger: () => {} });
  });

  afterEach(() => {
    configureValidator({
      enabled: process.env.NODE_ENV !== 'production',
      strict: false,
      logRawData: false,
      logger: undefined,
    });
  });

  test('should pass validation when disabled', () => {
    configureValidator({ enabled: false });

    const mcpResult = {
      content: [
        { type: 'text', text: JSON.stringify({ bad: 'data' }) },
      ],
    };

    const result = validateMcpToolResponse('get_activities', mcpResult);
    expect(result.valid).toBe(true);
  });

  test('should skip validation for error responses', () => {
    const mcpResult = {
      isError: true,
      content: [
        { type: 'text', text: 'Something went wrong' },
      ],
    };

    const result = validateMcpToolResponse('get_activities', mcpResult);
    expect(result.valid).toBe(true);
  });

  test('should pass for unknown tools (no schema)', () => {
    const mcpResult = {
      content: [
        { type: 'text', text: JSON.stringify({ anything: 'goes' }) },
      ],
    };

    const result = validateMcpToolResponse('unknown_tool_xyz', mcpResult);
    expect(result.valid).toBe(true);
  });

  test('should throw in strict mode for invalid responses', () => {
    configureValidator({ enabled: true, strict: true, logger: () => {} });

    const mcpResult = {
      content: [
        { type: 'text', text: JSON.stringify({ fitness_score: 999 }) },
      ],
    };

    expect(() => {
      validateMcpToolResponse('calculate_fitness_score', mcpResult);
    }).toThrow();
  });
});

describe('validateToolResponse (Zod Schema)', () => {
  test('should return success result with parsed data', () => {
    const response = {
      provider: 'strava',
      status: 'connected',
      connected: true,
    };

    const result = validateToolResponse('get_connection_status', response);
    expect(result.success).toBe(true);
    expect(result.data).toBeDefined();
    expect(result.data.provider).toBe('strava');
  });

  test('should return error result with validation issues', () => {
    const response = {
      fitness_score: -50,
    };

    const result = validateToolResponse('calculate_fitness_score', response);
    expect(result.success).toBe(false);
    expect(result.error).toBeDefined();
    expect(result.error.issues).toBeDefined();
    expect(result.error.issues.length).toBeGreaterThan(0);
  });

  test('should handle schemas with passthrough (extra fields allowed)', () => {
    const response = {
      activities: [],
      provider: 'strava',
      custom_field: 'should be allowed',
      another_extra: 42,
    };

    const result = validateToolResponse('get_activities', response);
    expect(result.success).toBe(true);
  });
});

describe('hasResponseSchema Coverage', () => {
  const toolsWithSchemas = [
    'connect_provider',
    'get_connection_status',
    'disconnect_provider',
    'get_activities',
    'get_athlete',
    'get_stats',
    'get_activity_intelligence',
    'analyze_activity',
    'calculate_metrics',
    'analyze_performance_trends',
    'compare_activities',
    'detect_patterns',
    'generate_recommendations',
    'calculate_fitness_score',
    'predict_performance',
    'analyze_training_load',
    'set_goal',
    'track_progress',
    'suggest_goals',
    'analyze_goal_feasibility',
    'calculate_daily_nutrition',
    'analyze_sleep_quality',
    'calculate_recovery_score',
  ];

  test.each(toolsWithSchemas)('should have schema for %s', (toolName) => {
    expect(hasResponseSchema(toolName)).toBe(true);
  });

  test('should return false for internal/meta tools', () => {
    expect(hasResponseSchema('connect_to_pierre')).toBe(false);
    expect(hasResponseSchema('internal_tool')).toBe(false);
  });
});
