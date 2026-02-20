// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for PierreError structured error types
// ABOUTME: Tests error construction, error codes, cause chains, and error handling patterns

const { PierreError, PierreErrorCode } = require('../../dist/index.js');

describe('PierreErrorCode Enum', () => {
  test('should define all expected error codes', () => {
    const expectedCodes = [
      'NETWORK_ERROR',
      'AUTH_ERROR',
      'CONFIG_ERROR',
      'STORAGE_ERROR',
      'TIMEOUT_ERROR',
      'VALIDATION_ERROR',
      'PROVIDER_ERROR',
    ];

    for (const code of expectedCodes) {
      expect(PierreErrorCode[code]).toBe(code);
    }
  });

  test('should have exactly 7 error codes', () => {
    const codes = Object.keys(PierreErrorCode);
    expect(codes).toHaveLength(7);
  });

  test('error codes should be string values matching their keys', () => {
    for (const [key, value] of Object.entries(PierreErrorCode)) {
      expect(key).toBe(value);
    }
  });
});

describe('PierreError Construction', () => {
  test('should create error with code and message', () => {
    const error = new PierreError(PierreErrorCode.NETWORK_ERROR, 'Connection failed');

    expect(error).toBeInstanceOf(Error);
    expect(error).toBeInstanceOf(PierreError);
    expect(error.code).toBe(PierreErrorCode.NETWORK_ERROR);
    expect(error.message).toBe('Connection failed');
    expect(error.name).toBe('PierreError');
  });

  test('should create error with optional cause', () => {
    const cause = new Error('socket timeout');
    const error = new PierreError(PierreErrorCode.TIMEOUT_ERROR, 'Request timed out', cause);

    expect(error.cause).toBe(cause);
    expect(error.cause.message).toBe('socket timeout');
  });

  test('should allow undefined cause', () => {
    const error = new PierreError(PierreErrorCode.AUTH_ERROR, 'Token expired');

    expect(error.cause).toBeUndefined();
  });

  test('should be catchable as a standard Error', () => {
    expect(() => {
      throw new PierreError(PierreErrorCode.CONFIG_ERROR, 'Invalid config');
    }).toThrow(Error);
  });

  test('should be catchable as a PierreError', () => {
    expect(() => {
      throw new PierreError(PierreErrorCode.STORAGE_ERROR, 'Keychain unavailable');
    }).toThrow(PierreError);
  });
});

describe('PierreError Error Codes Usage', () => {
  test('NETWORK_ERROR for connection failures', () => {
    const error = new PierreError(
      PierreErrorCode.NETWORK_ERROR,
      'Failed to connect to Pierre server at http://localhost:8081'
    );
    expect(error.code).toBe('NETWORK_ERROR');
  });

  test('AUTH_ERROR for authentication failures', () => {
    const error = new PierreError(
      PierreErrorCode.AUTH_ERROR,
      'OAuth token expired and refresh failed'
    );
    expect(error.code).toBe('AUTH_ERROR');
  });

  test('CONFIG_ERROR for configuration issues', () => {
    const error = new PierreError(
      PierreErrorCode.CONFIG_ERROR,
      'Missing required OAuth client credentials'
    );
    expect(error.code).toBe('CONFIG_ERROR');
  });

  test('STORAGE_ERROR for keychain/storage failures', () => {
    const error = new PierreError(
      PierreErrorCode.STORAGE_ERROR,
      'Failed to read tokens from OS keychain'
    );
    expect(error.code).toBe('STORAGE_ERROR');
  });

  test('TIMEOUT_ERROR for operation timeouts', () => {
    const error = new PierreError(
      PierreErrorCode.TIMEOUT_ERROR,
      'Token validation timed out after 3000ms'
    );
    expect(error.code).toBe('TIMEOUT_ERROR');
  });

  test('VALIDATION_ERROR for invalid data', () => {
    const error = new PierreError(
      PierreErrorCode.VALIDATION_ERROR,
      'Response schema validation failed for get_activities'
    );
    expect(error.code).toBe('VALIDATION_ERROR');
  });

  test('PROVIDER_ERROR for provider-specific failures', () => {
    const error = new PierreError(
      PierreErrorCode.PROVIDER_ERROR,
      'Strava API rate limit exceeded'
    );
    expect(error.code).toBe('PROVIDER_ERROR');
  });
});

describe('PierreError Cause Chain', () => {
  test('should support nested error causes for debugging', () => {
    const rootCause = new Error('ECONNREFUSED');
    const networkError = new PierreError(
      PierreErrorCode.NETWORK_ERROR,
      'HTTP request failed',
      rootCause
    );
    const authError = new PierreError(
      PierreErrorCode.AUTH_ERROR,
      'Token refresh failed',
      networkError
    );

    expect(authError.cause).toBe(networkError);
    expect(authError.cause.cause).toBe(rootCause);
    expect(authError.cause.cause.message).toBe('ECONNREFUSED');
  });

  test('should preserve stack trace', () => {
    const error = new PierreError(PierreErrorCode.NETWORK_ERROR, 'Test error');
    expect(error.stack).toBeDefined();
    expect(error.stack).toContain('PierreError');
  });
});

describe('PierreError Type Discrimination', () => {
  test('should allow switching on error code', () => {
    const error = new PierreError(PierreErrorCode.AUTH_ERROR, 'Token expired');

    let handled = false;
    switch (error.code) {
      case PierreErrorCode.AUTH_ERROR:
        handled = true;
        break;
      case PierreErrorCode.NETWORK_ERROR:
        handled = false;
        break;
      default:
        handled = false;
    }

    expect(handled).toBe(true);
  });

  test('should be distinguishable from generic errors via instanceof', () => {
    const pierreError = new PierreError(PierreErrorCode.AUTH_ERROR, 'auth failed');
    const genericError = new Error('generic failure');

    expect(pierreError instanceof PierreError).toBe(true);
    expect(genericError instanceof PierreError).toBe(false);
  });
});
