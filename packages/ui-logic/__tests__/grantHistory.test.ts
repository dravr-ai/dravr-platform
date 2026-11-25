// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for how Connected apps reads a receipt and a policy — each (event, actor) pair's outcome, the receipt's app, who is always allowed
// ABOUTME: A renewal is an issue by the AS citing a policy; a baseline issue by the AS reads as allowed (carnet#1544)

import { describe, it, expect } from 'vitest';
import type { OAuthGrant, OAuthGrantReceipt, OAuthStandingPolicy } from '@pierre/shared-types';
import { alwaysAllowedClients, clientNames, grantReceiptClientId, grantReceiptOutcome } from '../src/grantHistory';

function receipt(
  event: OAuthGrantReceipt['event'],
  actor: OAuthGrantReceipt['actor'],
  approval: Record<string, unknown> = {},
): OAuthGrantReceipt {
  return {
    id: 'r1',
    client_grant_id: 'g1',
    event,
    actor,
    at: '2026-11-25T12:00:00Z',
    payload: { grant: { client_id: 'https://client.example/meta.json' }, approval: { actor, ...approval } },
    payload_sha256: 'ab',
    jws: null,
    logged_at: null,
  };
}

const grant: OAuthGrant = {
  id: 'g1',
  client_id: 'c1',
  client_name: 'Claude',
  client_host: 'claude.ai',
  scope: 'fitness:read',
  granted_at: '2026-11-01T00:00:00Z',
  preset: 'training_coach',
  max_tier: 'T1',
  expires_at: null,
};

const policy: OAuthStandingPolicy = {
  id: 'p1',
  client_id: 'c2',
  client_name: 'Coach app',
  client_host: 'coach.example',
  preset: 'sleep_coach',
  created_at: '2026-11-02T00:00:00Z',
};

describe('grantReceiptOutcome', () => {
  it('names every event and actor pair the server writes', () => {
    expect(grantReceiptOutcome(receipt('issue', 'user'))).toBe('allowed');
    expect(grantReceiptOutcome(receipt('issue', 'auto', { standing_policy_ref: { id: 'p1' } }))).toBe('renewed');
    expect(grantReceiptOutcome(receipt('issue', 'auto', { standing_policy_ref: null, baseline: true }))).toBe('allowed');
    expect(grantReceiptOutcome(receipt('revoke', 'user'))).toBe('endedByYou');
    expect(grantReceiptOutcome(receipt('revoke', 'client'))).toBe('endedByApp');
    expect(grantReceiptOutcome(receipt('revoke', 'auto'))).toBe('endedByDravr');
    expect(grantReceiptOutcome(receipt('expire', 'auto'))).toBe('expired');
  });

  it('reads a payload without an approval member as allowed', () => {
    expect(grantReceiptOutcome({ ...receipt('issue', 'auto'), payload: {} })).toBe('allowed');
  });
});

describe('grantReceiptClientId', () => {
  it('reads the client from the signed payload, and none from a payload without one', () => {
    expect(grantReceiptClientId(receipt('issue', 'user'))).toBe('https://client.example/meta.json');
    expect(grantReceiptClientId({ ...receipt('issue', 'user'), payload: { grant: { client_id: '' } } })).toBeNull();
    expect(grantReceiptClientId({ ...receipt('issue', 'user'), payload: { grant: 'c1' } })).toBeNull();
  });
});

describe('clientNames and alwaysAllowedClients', () => {
  it('names each client by its grants and policies, and lists the policies clients', () => {
    const names = clientNames([grant, { ...grant, client_id: 'c3', client_name: null }], [policy]);
    expect([...names.entries()]).toEqual([
      ['c1', 'Claude'],
      ['c2', 'Coach app'],
    ]);
    expect([...alwaysAllowedClients([policy])]).toEqual(['c2']);
  });
});
