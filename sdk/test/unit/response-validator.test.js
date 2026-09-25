// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for response validation utilities and configuration
// ABOUTME: Tests validator configuration, stats tracking, and structuredContent checks against outputSchemas

const {
  configureValidator,
  getValidatorConfig,
  getValidationStats,
  resetValidationStats,
  validateMcpToolResponse,
  isValidResponse,
  validateWithStats,
  hasResponseSchema,
  registerToolOutputSchemas,
} = require('../../dist/index.js');

/**
 * Shaped like the server's derived outputSchemas: JSON Schema 2020-12, nested
 * types under `$defs`, and a `Formatted<T>` answer as an `anyOf` over the
 * tool's own shape and the TOON envelope — with the root `"type": "object"`
 * the server now always sets.
 */
const ACTIVITIES_SCHEMA = {
  $schema: 'https://json-schema.org/draft/2020-12/schema',
  title: 'Formatted_GetActivitiesResult',
  type: 'object',
  anyOf: [
    { $ref: '#/$defs/GetActivitiesResult' },
    {
      type: 'object',
      properties: { toon: { type: 'string' }, format: { type: 'string' } },
      required: ['toon', 'format'],
    },
  ],
  $defs: {
    GetActivitiesResult: {
      type: 'object',
      properties: {
        provider: { type: 'string' },
        count: { type: 'integer', format: 'uint32', minimum: 0 },
        activities: { type: 'array', items: { $ref: '#/$defs/Activity' } },
      },
      required: ['provider', 'count', 'activities'],
    },
    Activity: {
      type: 'object',
      properties: {
        id: { type: 'string' },
        distance_meters: { type: ['number', 'null'], format: 'double' },
      },
      required: ['id', 'distance_meters'],
    },
  },
};

const DISCONNECT_SCHEMA = {
  $schema: 'https://json-schema.org/draft/2020-12/schema',
  title: 'DisconnectProviderResult',
  type: 'object',
  properties: {
    provider: { type: 'string' },
    status: { type: 'string' },
    message: { type: 'string' },
  },
  required: ['provider', 'status', 'message'],
};

const LISTING = [
  { name: 'get_activities', outputSchema: ACTIVITIES_SCHEMA },
  { name: 'disconnect_provider', outputSchema: DISCONNECT_SCHEMA },
  { name: 'tool_without_output_schema' },
];

function toolResult(structuredContent) {
  return {
    content: [{ type: 'text', text: JSON.stringify(structuredContent) }],
    structuredContent,
  };
}

const VALID_ACTIVITIES = {
  provider: 'strava',
  count: 1,
  activities: [{ id: '9001', distance_meters: 32000 }],
};

function resetValidator() {
  configureValidator({
    enabled: process.env.NODE_ENV !== 'production',
    strict: false,
    logRawData: false,
    logger: undefined,
  });
  registerToolOutputSchemas([]);
}

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

describe('registerToolOutputSchemas', () => {
  beforeEach(() => {
    configureValidator({ enabled: true, strict: false, logger: () => {} });
  });
  afterEach(resetValidator);

  test('adopts the outputSchemas a tools/list result carries', () => {
    registerToolOutputSchemas(LISTING);

    expect(hasResponseSchema('get_activities')).toBe(true);
    expect(hasResponseSchema('disconnect_provider')).toBe(true);
    expect(hasResponseSchema('tool_without_output_schema')).toBe(false);
  });

  test('a later listing replaces the earlier one', () => {
    registerToolOutputSchemas(LISTING);
    registerToolOutputSchemas([{ name: 'disconnect_provider', outputSchema: DISCONNECT_SCHEMA }]);

    expect(hasResponseSchema('get_activities')).toBe(false);
    expect(hasResponseSchema('disconnect_provider')).toBe(true);
  });

  test('a schema that does not compile is logged and left out', () => {
    const messages = [];
    configureValidator({ logger: (msg) => messages.push(msg) });

    registerToolOutputSchemas([
      { name: 'broken', outputSchema: { type: 'object', properties: { a: { $ref: '#/$defs/Missing' } } } },
      { name: 'disconnect_provider', outputSchema: DISCONNECT_SCHEMA },
    ]);

    expect(hasResponseSchema('broken')).toBe(false);
    expect(hasResponseSchema('disconnect_provider')).toBe(true);
    expect(messages.some((m) => m.includes('"broken"'))).toBe(true);
  });
});

describe('validateMcpToolResponse', () => {
  beforeEach(() => {
    configureValidator({ enabled: true, strict: false, logger: () => {} });
    registerToolOutputSchemas(LISTING);
  });
  afterEach(resetValidator);

  test('accepts structuredContent matching the tool shape, through $refs', () => {
    const validated = validateMcpToolResponse('get_activities', toolResult(VALID_ACTIVITIES));

    expect(validated.valid).toBe(true);
    expect(validated.data).toEqual(VALID_ACTIVITIES);
    expect(isValidResponse(validated)).toBe(true);
  });

  test('accepts the TOON arm of a Formatted answer', () => {
    const validated = validateMcpToolResponse(
      'get_activities',
      toolResult({ toon: 'provider: strava', format: 'toon' }),
    );

    expect(validated.valid).toBe(true);
  });

  test('refuses structuredContent that breaks the schema, naming where', () => {
    const validated = validateMcpToolResponse(
      'get_activities',
      toolResult({ provider: 'strava', count: 1, activities: [{ id: 9001 }] }),
    );

    expect(validated.valid).toBe(false);
    expect(isValidResponse(validated)).toBe(false);
    expect(validated.errors.some((e) => e.startsWith('/activities/0'))).toBe(true);
  });

  test('a result without structuredContent fails when the tool declares a schema', () => {
    const validated = validateMcpToolResponse('disconnect_provider', {
      content: [{ type: 'text', text: '{"message":"Disconnected from strava"}' }],
    });

    expect(validated.valid).toBe(false);
    expect(validated.errors).toEqual([
      'has an output schema but did not return structured content',
    ]);
  });

  test('error results are not validated', () => {
    const validated = validateMcpToolResponse('disconnect_provider', {
      isError: true,
      content: [{ type: 'text', text: 'Something went wrong' }],
    });

    expect(validated.valid).toBe(true);
  });

  test('a tool without an outputSchema cannot be validated and passes', () => {
    const validated = validateMcpToolResponse('tool_without_output_schema', {
      content: [{ type: 'text', text: 'anything' }],
    });

    expect(validated.valid).toBe(true);
  });

  test('passes everything while disabled', () => {
    configureValidator({ enabled: false });

    const validated = validateMcpToolResponse('disconnect_provider', toolResult({ bad: 'data' }));
    expect(validated.valid).toBe(true);
  });

  test('throws in strict mode', () => {
    configureValidator({ strict: true });

    expect(() => validateMcpToolResponse('disconnect_provider', toolResult({ provider: 'strava' }))).toThrow(
      /disconnect_provider/,
    );
  });
});

describe('Validation Statistics', () => {
  beforeEach(() => {
    resetValidationStats();
    configureValidator({ enabled: true, strict: false, logger: () => {} });
    registerToolOutputSchemas(LISTING);
  });
  afterEach(resetValidator);

  test('should start with zero stats after reset', () => {
    const stats = getValidationStats();

    expect(stats.totalCalls).toBe(0);
    expect(stats.validResponses).toBe(0);
    expect(stats.invalidResponses).toBe(0);
    expect(stats.skippedResponses).toBe(0);
  });

  test('counts valid, invalid and skipped results', () => {
    validateWithStats('get_activities', toolResult(VALID_ACTIVITIES));
    validateWithStats('disconnect_provider', toolResult({ provider: 'strava' }));
    validateWithStats('tool_without_output_schema', { content: [] });

    const stats = getValidationStats();
    expect(stats.totalCalls).toBe(3);
    expect(stats.validResponses).toBe(1);
    expect(stats.invalidResponses).toBe(1);
    expect(stats.skippedResponses).toBe(1);
    expect(stats.errorsByTool).toEqual({ disconnect_provider: 1 });
  });

  test('should reset stats correctly', () => {
    validateWithStats('get_activities', toolResult(VALID_ACTIVITIES));
    expect(getValidationStats().totalCalls).toBe(1);

    resetValidationStats();
    const stats = getValidationStats();
    expect(stats.totalCalls).toBe(0);
    expect(stats.validResponses).toBe(0);
    expect(stats.invalidResponses).toBe(0);
  });
});
