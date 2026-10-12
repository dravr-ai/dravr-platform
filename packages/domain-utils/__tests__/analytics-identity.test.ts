// ABOUTME: Tests the PostHog distinct id the web and phone derive from the user UUID
// ABOUTME: Pins the same hex as the server's hash_id so a coach's client and server events join

import { describe, it, expect, vi } from 'vitest';
import { analyticsDistinctId, type Sha256Digest } from '../src/analytics-identity';

// The same pair is pinned by `hash_id` in crates/pierre-services/src/analytics.rs
// and by the web and phone analytics tests: the lowercase hex SHA-256 of the UUID.
const USER_ID = '123e4567-e89b-12d3-a456-426614174000';
const DISTINCT_ID = '986c0dc956dc822b5d8f698661b9eb1ef880786ff9043c16744d2a420e99e9bb';

const webCryptoSha256: Sha256Digest = (data) => crypto.subtle.digest('SHA-256', data);

describe('analyticsDistinctId', () => {
  it('is the lowercase hex SHA-256 of the user UUID, with no prefix', async () => {
    await expect(analyticsDistinctId(USER_ID, webCryptoSha256)).resolves.toBe(DISTINCT_ID);
  });

  it('hands the platform the UTF-8 bytes of the id, as the server hashes them', async () => {
    const sha256 = vi.fn<Sha256Digest>(async () => new Uint8Array(32).buffer);

    await analyticsDistinctId('é-1', sha256);

    expect(sha256).toHaveBeenCalledTimes(1);
    expect(Array.from(sha256.mock.calls[0][0])).toEqual([0xc3, 0xa9, 0x2d, 0x31]);
  });

  it('writes every digest byte as two hex digits', async () => {
    const digest = Uint8Array.from([0x00, 0x0a, 0xff, 0x10]).buffer;

    await expect(analyticsDistinctId(USER_ID, async () => digest)).resolves.toBe('000aff10');
  });

  it('passes a failing digest through rather than inventing an id', async () => {
    const failing: Sha256Digest = async () => {
      throw new Error('digest unavailable');
    };

    await expect(analyticsDistinctId(USER_ID, failing)).rejects.toThrow('digest unavailable');
  });
});
