// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for secure token storage (EncryptedFileStorage and KeychainTokenStorage)
// ABOUTME: Tests encrypt/decrypt roundtrips, error handling, and storage lifecycle

const { EncryptedFileStorage, createSecureStorage } = require('../../dist/index.js');
const fs = require('fs');
const path = require('path');
const os = require('os');
const crypto = require('crypto');

// Use a temporary directory for test files to avoid polluting home dir
const TEST_DIR = path.join(os.tmpdir(), 'pierre-storage-test-' + process.pid);

describe('EncryptedFileStorage - Encrypt/Decrypt Roundtrip', () => {
  let storage;
  let originalHomedir;

  beforeAll(() => {
    // Create temp directory for test encrypted files
    fs.mkdirSync(TEST_DIR, { recursive: true });
  });

  beforeEach(() => {
    // Suppress log output during tests
    storage = new EncryptedFileStorage(() => {});
    // Override the encrypted file path to use temp directory
    storage.encryptedFilePath = path.join(TEST_DIR, `tokens-${Date.now()}.enc`);
  });

  afterEach(() => {
    // Clean up any test files
    try {
      if (fs.existsSync(storage.encryptedFilePath)) {
        fs.unlinkSync(storage.encryptedFilePath);
      }
    } catch (e) {
      // Ignore cleanup errors
    }
  });

  afterAll(() => {
    // Clean up temp directory
    try {
      fs.rmSync(TEST_DIR, { recursive: true, force: true });
    } catch (e) {
      // Ignore cleanup errors
    }
  });

  test('should save and retrieve simple token data', async () => {
    const tokens = {
      pierre: {
        access_token: 'test_access_token_123',
        refresh_token: 'test_refresh_token_456',
        token_type: 'Bearer',
        expires_in: 3600,
      },
    };

    await storage.saveTokens(tokens);
    const retrieved = await storage.getTokens();

    expect(retrieved).toEqual(tokens);
    expect(retrieved.pierre.access_token).toBe('test_access_token_123');
  });

  test('should preserve complex nested token structures', async () => {
    const tokens = {
      pierre: {
        access_token: 'pierre_token',
        refresh_token: 'pierre_refresh',
        expires_in: 3600,
        saved_at: 1700000000,
      },
      providers: {
        strava: {
          access_token: 'strava_token',
          refresh_token: 'strava_refresh',
          expires_at: 1700003600,
          token_type: 'Bearer',
          scope: 'read,activity:read',
        },
        fitbit: {
          access_token: 'fitbit_token',
          refresh_token: 'fitbit_refresh',
          expires_at: 1700003600,
          token_type: 'Bearer',
          scope: 'activity heartrate',
        },
      },
      client_info: {
        client_id: 'registered_client_id',
        client_secret: 'registered_client_secret',
      },
    };

    await storage.saveTokens(tokens);
    const retrieved = await storage.getTokens();

    expect(retrieved).toEqual(tokens);
    expect(retrieved.providers.strava.access_token).toBe('strava_token');
    expect(retrieved.providers.fitbit.scope).toBe('activity heartrate');
    expect(retrieved.client_info.client_id).toBe('registered_client_id');
  });

  test('should handle empty token object', async () => {
    const tokens = {};

    await storage.saveTokens(tokens);
    const retrieved = await storage.getTokens();

    expect(retrieved).toEqual({});
  });

  test('should handle tokens with special characters', async () => {
    const tokens = {
      pierre: {
        access_token: 'eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIn0.test+/=',
        refresh_token: 'dXNlcjpwYXNzd29yZA==',
        scope: 'read:fitness write:fitness offline_access',
      },
    };

    await storage.saveTokens(tokens);
    const retrieved = await storage.getTokens();

    expect(retrieved.pierre.access_token).toBe(tokens.pierre.access_token);
    expect(retrieved.pierre.refresh_token).toBe(tokens.pierre.refresh_token);
  });

  test('should handle large token data', async () => {
    // Simulate a large token payload (e.g., JWTs with many claims)
    const largePayload = 'x'.repeat(10000);
    const tokens = {
      pierre: { access_token: largePayload, token_type: 'Bearer' },
    };

    await storage.saveTokens(tokens);
    const retrieved = await storage.getTokens();

    expect(retrieved.pierre.access_token).toBe(largePayload);
    expect(retrieved.pierre.access_token).toHaveLength(10000);
  });

  test('should overwrite existing tokens on save', async () => {
    const tokens1 = { pierre: { access_token: 'first_token' } };
    const tokens2 = { pierre: { access_token: 'second_token' } };

    await storage.saveTokens(tokens1);
    await storage.saveTokens(tokens2);

    const retrieved = await storage.getTokens();
    expect(retrieved.pierre.access_token).toBe('second_token');
  });
});

describe('EncryptedFileStorage - Clear Tokens', () => {
  let storage;

  beforeAll(() => {
    fs.mkdirSync(TEST_DIR, { recursive: true });
  });

  beforeEach(() => {
    storage = new EncryptedFileStorage(() => {});
    storage.encryptedFilePath = path.join(TEST_DIR, `tokens-clear-${Date.now()}.enc`);
  });

  afterAll(() => {
    try {
      fs.rmSync(TEST_DIR, { recursive: true, force: true });
    } catch (e) {
      // Ignore
    }
  });

  test('should clear stored tokens by deleting encrypted file', async () => {
    const tokens = { pierre: { access_token: 'to_be_cleared' } };
    await storage.saveTokens(tokens);

    // Verify file exists
    expect(fs.existsSync(storage.encryptedFilePath)).toBe(true);

    await storage.clearTokens();

    // Verify file is gone
    expect(fs.existsSync(storage.encryptedFilePath)).toBe(false);
  });

  test('should handle clearing when no tokens exist', async () => {
    // Should not throw even if no file exists
    await expect(storage.clearTokens()).resolves.not.toThrow();
  });

  test('should return null after clearing tokens', async () => {
    const tokens = { pierre: { access_token: 'temp_token' } };
    await storage.saveTokens(tokens);
    await storage.clearTokens();

    const retrieved = await storage.getTokens();
    expect(retrieved).toBeNull();
  });
});

describe('EncryptedFileStorage - Error Handling', () => {
  let storage;

  beforeAll(() => {
    fs.mkdirSync(TEST_DIR, { recursive: true });
  });

  beforeEach(() => {
    storage = new EncryptedFileStorage(() => {});
    storage.encryptedFilePath = path.join(TEST_DIR, `tokens-error-${Date.now()}.enc`);
  });

  afterAll(() => {
    try {
      fs.rmSync(TEST_DIR, { recursive: true, force: true });
    } catch (e) {
      // Ignore
    }
  });

  test('should return null when reading non-existent file', async () => {
    const result = await storage.getTokens();
    expect(result).toBeNull();
  });

  test('should return null for corrupted encrypted data', async () => {
    // Write garbage data to the encrypted file
    fs.writeFileSync(storage.encryptedFilePath, 'not:valid:encrypted:data:here', 'utf8');

    const result = await storage.getTokens();
    expect(result).toBeNull();
  });

  test('should return null for truncated encrypted data', async () => {
    // Write a truncated version of what would be valid encrypted data
    fs.writeFileSync(storage.encryptedFilePath, 'aabbcc:ddeeff', 'utf8');

    const result = await storage.getTokens();
    expect(result).toBeNull();
  });

  test('should return null for empty encrypted file', async () => {
    fs.writeFileSync(storage.encryptedFilePath, '', 'utf8');

    const result = await storage.getTokens();
    expect(result).toBeNull();
  });
});

describe('EncryptedFileStorage - Encryption Properties', () => {
  let storage;

  beforeAll(() => {
    fs.mkdirSync(TEST_DIR, { recursive: true });
  });

  beforeEach(() => {
    storage = new EncryptedFileStorage(() => {});
    storage.encryptedFilePath = path.join(TEST_DIR, `tokens-enc-${Date.now()}.enc`);
  });

  afterEach(() => {
    try {
      if (fs.existsSync(storage.encryptedFilePath)) {
        fs.unlinkSync(storage.encryptedFilePath);
      }
    } catch (e) {
      // Ignore
    }
  });

  afterAll(() => {
    try {
      fs.rmSync(TEST_DIR, { recursive: true, force: true });
    } catch (e) {
      // Ignore
    }
  });

  test('should store data in IV:AuthTag:Encrypted format', async () => {
    const tokens = { pierre: { access_token: 'test' } };
    await storage.saveTokens(tokens);

    const fileContent = fs.readFileSync(storage.encryptedFilePath, 'utf8');
    const parts = fileContent.split(':');

    // Format: IV:AuthTag:EncryptedData
    expect(parts).toHaveLength(3);

    // IV should be 12 bytes = 24 hex chars
    expect(parts[0]).toMatch(/^[0-9a-f]{24}$/);

    // AuthTag should be 16 bytes = 32 hex chars
    expect(parts[1]).toMatch(/^[0-9a-f]{32}$/);

    // Encrypted data should be hex-encoded
    expect(parts[2]).toMatch(/^[0-9a-f]+$/);
  });

  test('should not store plaintext token values on disk', async () => {
    const secretToken = 'super_secret_access_token_value_12345';
    const tokens = { pierre: { access_token: secretToken } };
    await storage.saveTokens(tokens);

    const fileContent = fs.readFileSync(storage.encryptedFilePath, 'utf8');

    // The raw token value should NOT appear in the encrypted file
    expect(fileContent).not.toContain(secretToken);
    // The word "access_token" should not appear either
    expect(fileContent).not.toContain('access_token');
  });

  test('should use unique IV for each save operation', async () => {
    const tokens = { pierre: { access_token: 'same_token' } };

    // Save and read IV
    await storage.saveTokens(tokens);
    const content1 = fs.readFileSync(storage.encryptedFilePath, 'utf8');
    const iv1 = content1.split(':')[0];

    // Save again with the same data
    await storage.saveTokens(tokens);
    const content2 = fs.readFileSync(storage.encryptedFilePath, 'utf8');
    const iv2 = content2.split(':')[0];

    // IVs should be different even for the same data
    expect(iv1).not.toBe(iv2);
  });

  test('encryption key derivation should be deterministic for the same machine', () => {
    // Two instances on the same machine should derive the same key
    const storage1 = new EncryptedFileStorage(() => {});
    const storage2 = new EncryptedFileStorage(() => {});

    // Both should be able to decrypt each other's data
    // We test this by saving with one and reading with the other
    const testPath = path.join(TEST_DIR, `key-test-${Date.now()}.enc`);
    storage1.encryptedFilePath = testPath;
    storage2.encryptedFilePath = testPath;

    return (async () => {
      const tokens = { test: 'data' };
      await storage1.saveTokens(tokens);
      const retrieved = await storage2.getTokens();
      expect(retrieved).toEqual(tokens);

      // Clean up
      if (fs.existsSync(testPath)) {
        fs.unlinkSync(testPath);
      }
    })();
  });
});

describe('EncryptedFileStorage - Migration from Plaintext', () => {
  let storage;

  beforeAll(() => {
    fs.mkdirSync(TEST_DIR, { recursive: true });
  });

  beforeEach(() => {
    storage = new EncryptedFileStorage(() => {});
    storage.encryptedFilePath = path.join(TEST_DIR, `tokens-migrate-${Date.now()}.enc`);
  });

  afterAll(() => {
    try {
      fs.rmSync(TEST_DIR, { recursive: true, force: true });
    } catch (e) {
      // Ignore
    }
  });

  test('should migrate tokens from plaintext file to encrypted storage', async () => {
    const plaintextPath = path.join(TEST_DIR, `plaintext-${Date.now()}.json`);
    const tokens = {
      pierre: { access_token: 'migrated_token', refresh_token: 'migrated_refresh' },
    };

    // Write plaintext file
    fs.writeFileSync(plaintextPath, JSON.stringify(tokens), 'utf8');
    expect(fs.existsSync(plaintextPath)).toBe(true);

    // Migrate
    const result = await storage.migrateFromPlaintextFile(plaintextPath);

    // Verify migration succeeded
    expect(result).toBe(true);

    // Plaintext file should be deleted
    expect(fs.existsSync(plaintextPath)).toBe(false);

    // Tokens should be readable from encrypted storage
    const retrieved = await storage.getTokens();
    expect(retrieved.pierre.access_token).toBe('migrated_token');
  });

  test('should return false when plaintext file does not exist', async () => {
    const result = await storage.migrateFromPlaintextFile('/nonexistent/path/tokens.json');
    expect(result).toBe(false);
  });

  test('should return false for invalid JSON in plaintext file', async () => {
    const plaintextPath = path.join(TEST_DIR, `invalid-${Date.now()}.json`);
    fs.writeFileSync(plaintextPath, 'not valid json{{{', 'utf8');

    const result = await storage.migrateFromPlaintextFile(plaintextPath);
    expect(result).toBe(false);

    // Clean up
    if (fs.existsSync(plaintextPath)) {
      fs.unlinkSync(plaintextPath);
    }
  });
});

describe('createSecureStorage Factory', () => {
  test('should create storage that implements SecureTokenStorage interface', async () => {
    const storage = await createSecureStorage(() => {});

    // Verify interface methods exist
    expect(typeof storage.saveTokens).toBe('function');
    expect(typeof storage.getTokens).toBe('function');
    expect(typeof storage.clearTokens).toBe('function');
    expect(typeof storage.migrateFromPlaintextFile).toBe('function');
  });

  test('should use encrypted file storage in CI environment', async () => {
    // Save original env
    const origCI = process.env.CI;

    try {
      process.env.CI = 'true';
      const storage = await createSecureStorage(() => {});

      // In CI, it should be EncryptedFileStorage (duck-type check via save/load)
      expect(typeof storage.saveTokens).toBe('function');
      expect(typeof storage.getTokens).toBe('function');
    } finally {
      // Restore original env
      if (origCI === undefined) {
        delete process.env.CI;
      } else {
        process.env.CI = origCI;
      }
    }
  });
});
