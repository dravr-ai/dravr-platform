// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the control process's seeded-session codes: each redeems its pair once, within its TTL, and never again
// ABOUTME: Also pins the Host headers the control process answers. Runs with `bun test` in the mobile unit lane (no install needed)

import { describe, expect, it } from 'bun:test';
import { isControlHost } from './control-host';
import { SEEDED_SESSION_CODE_TTL_MS, SeededSessionCodes } from './seeded-session';

const PAIR = { access_token: 'minted-access', refresh_token: 'minted-refresh' };

/** A clock the test moves by hand. */
function clock(start = 1_000_000): { now: () => number; advance: (ms: number) => void } {
  let at = start;
  return { now: () => at, advance: (ms) => (at += ms) };
}

describe('a seeded-session code', () => {
  it('redeems the pair it was issued for', () => {
    const codes = new SeededSessionCodes();
    const code = codes.issue(PAIR);
    expect(codes.redeem(code)).toEqual(PAIR);
  });

  it('is single-use: a second redemption gets nothing', () => {
    const codes = new SeededSessionCodes();
    const code = codes.issue(PAIR);
    expect(codes.redeem(code)).toEqual(PAIR);
    expect(codes.redeem(code)).toBeUndefined();
  });

  it('is refused once its TTL has passed, and stays spent', () => {
    const time = clock();
    const codes = new SeededSessionCodes(time.now);
    const fresh = codes.issue(PAIR);
    const late = codes.issue(PAIR);

    time.advance(SEEDED_SESSION_CODE_TTL_MS - 1);
    expect(codes.redeem(fresh)).toEqual(PAIR);

    time.advance(1);
    expect(codes.redeem(late)).toBeUndefined();
    expect(codes.redeem(late)).toBeUndefined();
  });

  it('names nothing it never issued', () => {
    const codes = new SeededSessionCodes();
    codes.issue(PAIR);
    expect(codes.redeem('')).toBeUndefined();
    expect(codes.redeem('not-a-code')).toBeUndefined();
  });

  it('is a different random string every time, carrying no token', () => {
    const codes = new SeededSessionCodes();
    const issued = new Set(Array.from({ length: 50 }, () => codes.issue(PAIR)));
    expect(issued.size).toBe(50);
    for (const code of issued) {
      expect(code).toMatch(/^[A-Za-z0-9_-]{22}$/);
      expect(code).not.toContain(PAIR.access_token);
    }
  });
});

describe('the control process Host check', () => {
  it('answers its own port on loopback and on the Android emulator alias', () => {
    expect(isControlHost('127.0.0.1:8098', 8098)).toBe(true);
    expect(isControlHost('localhost:8098', 8098)).toBe(true);
    expect(isControlHost('10.0.2.2:8098', 8098)).toBe(true);
  });

  it('refuses any other name, another port, a bare host, or no Host at all', () => {
    expect(isControlHost('attacker.example:8098', 8098)).toBe(false);
    expect(isControlHost('127.0.0.1.attacker.example:8098', 8098)).toBe(false);
    expect(isControlHost('127.0.0.1:8081', 8098)).toBe(false);
    expect(isControlHost('127.0.0.1', 8098)).toBe(false);
    expect(isControlHost(undefined, 8098)).toBe(false);
  });
});
