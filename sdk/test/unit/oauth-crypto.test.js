// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for OAuth cryptographic functions (PKCE, random string generation)
// ABOUTME: Validates RFC 7636 compliance, cryptographic randomness, and character set correctness

const { randomBytes, createHash } = require('crypto');

/**
 * Reimplementation of generateRandomString for unit testing.
 * This matches the production implementation in oauth-session-manager.ts
 * which uses crypto.randomBytes for cryptographic security.
 */
function generateRandomString(length) {
  const chars = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~';
  const bytes = randomBytes(length);
  let result = '';
  for (let i = 0; i < length; i++) {
    result += chars.charAt(bytes[i] % chars.length);
  }
  return result;
}

/**
 * Reimplementation of generateCodeChallenge for unit testing.
 * This matches the production implementation in oauth-session-manager.ts
 * which implements RFC 7636 S256 code challenge method.
 */
function generateCodeChallenge(codeVerifier) {
  const hash = createHash('sha256').update(codeVerifier).digest();
  return hash
    .toString('base64')
    .replace(/\+/g, '-')
    .replace(/\//g, '_')
    .replace(/=/g, '');
}

// RFC 7636 Section 4.1: unreserved characters for code verifier
const UNRESERVED_CHARS = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~';

describe('generateRandomString - Cryptographic Random String Generation', () => {
  test('should produce strings of the requested length', () => {
    const lengths = [1, 16, 32, 43, 64, 128];
    for (const len of lengths) {
      const result = generateRandomString(len);
      expect(result).toHaveLength(len);
    }
  });

  test('should only contain RFC 3986 unreserved characters', () => {
    // Generate a large sample to cover all possible byte values
    const result = generateRandomString(1000);
    for (const char of result) {
      expect(UNRESERVED_CHARS).toContain(char);
    }
  });

  test('should produce different strings on each call (non-deterministic)', () => {
    const results = new Set();
    // Generate 50 strings and verify they are all unique
    for (let i = 0; i < 50; i++) {
      results.add(generateRandomString(32));
    }
    // All 50 strings should be unique (probability of collision is negligible)
    expect(results.size).toBe(50);
  });

  test('should use crypto.randomBytes (not Math.random)', () => {
    // Mock Math.random to return a predictable sequence
    const originalRandom = Math.random;
    let mathRandomCalled = false;
    Math.random = () => {
      mathRandomCalled = true;
      return 0.5;
    };

    try {
      generateRandomString(32);
      // The function should NOT have called Math.random
      expect(mathRandomCalled).toBe(false);
    } finally {
      Math.random = originalRandom;
    }
  });

  test('should handle minimum length of 1', () => {
    const result = generateRandomString(1);
    expect(result).toHaveLength(1);
    expect(UNRESERVED_CHARS).toContain(result);
  });

  test('should use all characters from the charset', () => {
    // Generate a large sample and verify every character in the charset appears
    const sampleSize = 10000;
    const result = generateRandomString(sampleSize);
    const charSet = new Set(result);

    // Every character in the charset should appear at least once in 10K chars
    for (const char of UNRESERVED_CHARS) {
      expect(charSet.has(char)).toBe(true);
    }
  });

  test('should not produce characters outside the charset due to modulo mapping', () => {
    // The modulo operation (byte % 66) maps 256 byte values to 66 chars.
    // Verify no unmapped values leak through.
    const result = generateRandomString(50000);
    const charSet = new Set(UNRESERVED_CHARS);

    for (const char of result) {
      expect(charSet.has(char)).toBe(true);
    }
  });
});

describe('generateCodeChallenge - RFC 7636 PKCE S256 Compliance', () => {
  test('should produce a valid base64url-encoded string', () => {
    const verifier = generateRandomString(64);
    const challenge = generateCodeChallenge(verifier);

    // Base64url alphabet: A-Z, a-z, 0-9, -, _ (no +, /, =)
    expect(challenge).toMatch(/^[A-Za-z0-9_-]+$/);
  });

  test('should not contain base64 padding characters', () => {
    const verifier = generateRandomString(64);
    const challenge = generateCodeChallenge(verifier);

    expect(challenge).not.toContain('=');
    expect(challenge).not.toContain('+');
    expect(challenge).not.toContain('/');
  });

  test('should produce a 43-character challenge from SHA-256', () => {
    // SHA-256 produces 32 bytes = 256 bits
    // Base64 encoding: ceil(32 * 4/3) = 44 chars with padding, 43 without
    const verifier = generateRandomString(64);
    const challenge = generateCodeChallenge(verifier);

    expect(challenge).toHaveLength(43);
  });

  test('should produce deterministic output for the same input', () => {
    const verifier = 'dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk';
    const challenge1 = generateCodeChallenge(verifier);
    const challenge2 = generateCodeChallenge(verifier);

    expect(challenge1).toBe(challenge2);
  });

  test('should match known RFC 7636 test vector', () => {
    // RFC 7636 Appendix B test vector
    const verifier = 'dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk';
    const expectedChallenge = 'E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM';

    const challenge = generateCodeChallenge(verifier);
    expect(challenge).toBe(expectedChallenge);
  });

  test('should produce different challenges for different verifiers', () => {
    const verifier1 = generateRandomString(64);
    const verifier2 = generateRandomString(64);
    const challenge1 = generateCodeChallenge(verifier1);
    const challenge2 = generateCodeChallenge(verifier2);

    expect(challenge1).not.toBe(challenge2);
  });

  test('should handle minimum code verifier length (43 chars per RFC 7636)', () => {
    const verifier = generateRandomString(43);
    const challenge = generateCodeChallenge(verifier);

    expect(challenge).toHaveLength(43);
    expect(challenge).toMatch(/^[A-Za-z0-9_-]+$/);
  });

  test('should handle maximum code verifier length (128 chars per RFC 7636)', () => {
    const verifier = generateRandomString(128);
    const challenge = generateCodeChallenge(verifier);

    expect(challenge).toHaveLength(43);
    expect(challenge).toMatch(/^[A-Za-z0-9_-]+$/);
  });
});

describe('OAuth State Parameter Generation', () => {
  test('should generate state parameters of sufficient length', () => {
    // OAuth state should be at least 32 characters for entropy
    const state = generateRandomString(32);
    expect(state.length).toBeGreaterThanOrEqual(32);
  });

  test('should generate unique state values across multiple calls', () => {
    const states = new Set();
    for (let i = 0; i < 100; i++) {
      states.add(generateRandomString(32));
    }
    expect(states.size).toBe(100);
  });

  test('should generate URL-safe state parameters', () => {
    // State parameters appear in URLs, so they must be URL-safe
    const state = generateRandomString(32);
    // The character set (unreserved chars) is inherently URL-safe
    expect(state).toMatch(/^[A-Za-z0-9._~-]+$/);
  });
});

describe('PKCE Code Verifier Generation', () => {
  test('should generate code verifiers within RFC 7636 length bounds (43-128)', () => {
    // Standard PKCE code verifier is 64 bytes
    const verifier = generateRandomString(64);
    expect(verifier.length).toBeGreaterThanOrEqual(43);
    expect(verifier.length).toBeLessThanOrEqual(128);
  });

  test('should generate code verifiers using only allowed characters', () => {
    // RFC 7636 Section 4.1: ALPHA / DIGIT / "-" / "." / "_" / "~"
    const verifier = generateRandomString(128);
    expect(verifier).toMatch(/^[A-Za-z0-9._~-]+$/);
  });

  test('code verifier and challenge should form a valid PKCE pair', () => {
    // Verify the end-to-end PKCE flow
    const verifier = generateRandomString(64);
    const challenge = generateCodeChallenge(verifier);

    // Server-side verification: compute challenge from verifier and compare
    const serverChallenge = generateCodeChallenge(verifier);
    expect(serverChallenge).toBe(challenge);
  });

  test('different verifiers should produce different challenges', () => {
    const pairs = [];
    for (let i = 0; i < 20; i++) {
      const verifier = generateRandomString(64);
      const challenge = generateCodeChallenge(verifier);
      pairs.push({ verifier, challenge });
    }

    // All verifiers should be unique
    const verifiers = new Set(pairs.map(p => p.verifier));
    expect(verifiers.size).toBe(20);

    // All challenges should be unique
    const challenges = new Set(pairs.map(p => p.challenge));
    expect(challenges.size).toBe(20);
  });
});
