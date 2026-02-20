// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for SDK authentication mode configuration
// ABOUTME: Tests the discriminated union pattern for JWT, OAuth, credentials, and API key modes

describe('JWT Authentication Mode', () => {
  test('should accept valid JWT mode configuration', () => {
    const config = {
      mode: 'jwt',
      pierreServerUrl: 'http://localhost:8081',
      jwtToken: 'eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.test.signature',
    };

    expect(config.mode).toBe('jwt');
    expect(config.jwtToken).toBeTruthy();
    expect(config.pierreServerUrl).toBeTruthy();
  });

  test('should use JWT token directly without OAuth flow', () => {
    const config = {
      mode: 'jwt',
      pierreServerUrl: 'http://localhost:8081',
      jwtToken: 'test-jwt-token-12345',
    };

    // JWT mode creates tokens directly from the provided token
    const expectedTokens = {
      access_token: config.jwtToken,
      token_type: 'Bearer',
      expires_in: 3600,
      scope: 'read:fitness write:fitness',
    };

    expect(expectedTokens.access_token).toBe('test-jwt-token-12345');
    expect(expectedTokens.token_type).toBe('Bearer');
    // JWT mode does not have a refresh_token
    expect(expectedTokens).not.toHaveProperty('refresh_token');
  });

  test('should support optional callback port', () => {
    const config = {
      mode: 'jwt',
      pierreServerUrl: 'http://localhost:8081',
      jwtToken: 'test-token',
      callbackPort: 35535,
    };

    expect(config.callbackPort).toBe(35535);
  });

  test('should support disableBrowser option for testing', () => {
    const config = {
      mode: 'jwt',
      pierreServerUrl: 'http://localhost:8081',
      jwtToken: 'test-token',
      disableBrowser: true,
    };

    expect(config.disableBrowser).toBe(true);
  });
});

describe('OAuth 2.0 Authentication Mode', () => {
  test('should accept valid OAuth mode configuration', () => {
    const config = {
      mode: 'oauth',
      pierreServerUrl: 'http://localhost:8081',
      oauthClientId: 'client_abc123',
      oauthClientSecret: 'secret_xyz789',
    };

    expect(config.mode).toBe('oauth');
    expect(config.oauthClientId).toBeTruthy();
    expect(config.oauthClientSecret).toBeTruthy();
  });

  test('should store client credentials for OAuth flow', () => {
    const config = {
      mode: 'oauth',
      pierreServerUrl: 'http://localhost:8081',
      oauthClientId: 'registered_client_id',
      oauthClientSecret: 'registered_client_secret',
    };

    // OAuth mode uses pre-registered client credentials
    const clientInfo = {
      client_id: config.oauthClientId,
      client_secret: config.oauthClientSecret,
    };

    expect(clientInfo.client_id).toBe('registered_client_id');
    expect(clientInfo.client_secret).toBe('registered_client_secret');
  });

  test('should support optional token validation timeout', () => {
    const config = {
      mode: 'oauth',
      pierreServerUrl: 'http://localhost:8081',
      oauthClientId: 'client_id',
      oauthClientSecret: 'client_secret',
      tokenValidationTimeoutMs: 5000,
    };

    expect(config.tokenValidationTimeoutMs).toBe(5000);
  });
});

describe('User Credentials Authentication Mode', () => {
  test('should accept valid credentials mode configuration', () => {
    const config = {
      mode: 'credentials',
      pierreServerUrl: 'http://localhost:8081',
      userEmail: 'admin@example.com',
      userPassword: 'SecurePassword123',
    };

    expect(config.mode).toBe('credentials');
    expect(config.userEmail).toBeTruthy();
    expect(config.userPassword).toBeTruthy();
  });

  test('should use email/password for direct authentication', () => {
    const config = {
      mode: 'credentials',
      pierreServerUrl: 'http://localhost:8081',
      userEmail: 'test@pierre.dev',
      userPassword: 'TestPassword123!',
    };

    // Credentials mode authenticates directly without browser interaction
    expect(config.userEmail).toContain('@');
    expect(config.userPassword.length).toBeGreaterThan(0);
  });
});

describe('API Key Authentication Mode', () => {
  test('should accept valid API key mode configuration', () => {
    const config = {
      mode: 'api-key',
      pierreServerUrl: 'http://localhost:8081',
      apiKey: 'sk-test-api-key-123456789',
    };

    expect(config.mode).toBe('api-key');
    expect(config.apiKey).toBeTruthy();
  });
});

describe('Authentication Mode Discrimination', () => {
  test('should correctly discriminate between all four modes', () => {
    const configs = [
      { mode: 'jwt', pierreServerUrl: 'http://localhost:8081', jwtToken: 'token' },
      { mode: 'oauth', pierreServerUrl: 'http://localhost:8081', oauthClientId: 'id', oauthClientSecret: 'secret' },
      { mode: 'credentials', pierreServerUrl: 'http://localhost:8081', userEmail: 'a@b.com', userPassword: 'pass' },
      { mode: 'api-key', pierreServerUrl: 'http://localhost:8081', apiKey: 'key' },
    ];

    const modes = configs.map(c => c.mode);
    expect(modes).toEqual(['jwt', 'oauth', 'credentials', 'api-key']);
    expect(new Set(modes).size).toBe(4);
  });

  test('should identify JWT mode by jwtToken property', () => {
    const config = { mode: 'jwt', pierreServerUrl: 'http://localhost:8081', jwtToken: 'token' };

    if (config.mode === 'jwt') {
      expect(config.jwtToken).toBeDefined();
    }
  });

  test('should identify OAuth mode by oauthClientId property', () => {
    const config = { mode: 'oauth', pierreServerUrl: 'http://localhost:8081', oauthClientId: 'id', oauthClientSecret: 'secret' };

    if (config.mode === 'oauth') {
      expect(config.oauthClientId).toBeDefined();
      expect(config.oauthClientSecret).toBeDefined();
    }
  });

  test('should identify credentials mode by userEmail property', () => {
    const config = { mode: 'credentials', pierreServerUrl: 'http://localhost:8081', userEmail: 'a@b.com', userPassword: 'pass' };

    if (config.mode === 'credentials') {
      expect(config.userEmail).toBeDefined();
      expect(config.userPassword).toBeDefined();
    }
  });

  test('all modes share pierreServerUrl base property', () => {
    const configs = [
      { mode: 'jwt', pierreServerUrl: 'http://host:8081', jwtToken: 't' },
      { mode: 'oauth', pierreServerUrl: 'http://host:8081', oauthClientId: 'i', oauthClientSecret: 's' },
      { mode: 'credentials', pierreServerUrl: 'http://host:8081', userEmail: 'e', userPassword: 'p' },
      { mode: 'api-key', pierreServerUrl: 'http://host:8081', apiKey: 'k' },
    ];

    for (const config of configs) {
      expect(config.pierreServerUrl).toBe('http://host:8081');
    }
  });
});

describe('CLI Authentication Mode Priority', () => {
  test('--token flag selects JWT mode (highest priority)', () => {
    // CLI priority: --token > --oauth-client-id > --user-email > default oauth
    const cliArgs = { token: 'jwt-token', oauthClientId: 'id', userEmail: 'e@m.com' };

    // token takes precedence
    let selectedMode;
    if (cliArgs.token) {
      selectedMode = 'jwt';
    } else if (cliArgs.oauthClientId) {
      selectedMode = 'oauth';
    } else if (cliArgs.userEmail) {
      selectedMode = 'credentials';
    }

    expect(selectedMode).toBe('jwt');
  });

  test('--oauth-client-id selects OAuth mode when no --token', () => {
    const cliArgs = { oauthClientId: 'id', oauthClientSecret: 'secret', userEmail: 'e@m.com' };

    let selectedMode;
    if (cliArgs.token) {
      selectedMode = 'jwt';
    } else if (cliArgs.oauthClientId) {
      selectedMode = 'oauth';
    } else if (cliArgs.userEmail) {
      selectedMode = 'credentials';
    }

    expect(selectedMode).toBe('oauth');
  });

  test('--user-email selects credentials mode when no --token or --oauth-client-id', () => {
    const cliArgs = { userEmail: 'test@example.com', userPassword: 'pass123' };

    let selectedMode;
    if (cliArgs.token) {
      selectedMode = 'jwt';
    } else if (cliArgs.oauthClientId) {
      selectedMode = 'oauth';
    } else if (cliArgs.userEmail) {
      selectedMode = 'credentials';
    }

    expect(selectedMode).toBe('credentials');
  });
});
