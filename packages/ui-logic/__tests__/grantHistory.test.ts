// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for how Connected apps reads a receipt and a policy — each (event, actor) pair's outcome, the receipt's app, who is always allowed
// ABOUTME: A renewal is an issue by the AS citing a policy; a baseline issue by the AS reads as allowed (carnet#1544)

import { describe, it, expect } from 'vitest';
import type { OAuthGrant, OAuthGrantReceipt, OAuthStandingPolicy } from '@pierre/shared-types';
import de from '../../i18n/src/locales/de/translation.json';
import en from '../../i18n/src/locales/en/translation.json';
import es from '../../i18n/src/locales/es/translation.json';
import fr from '../../i18n/src/locales/fr/translation.json';
import pt from '../../i18n/src/locales/pt/translation.json';
import {
  CONNECTED_APP_PRESET_KEYS,
  GRANT_RECEIPT_OUTCOME_KEYS,
  alwaysAllowedClients,
  clientNames,
  connectedAppPresetLabel,
  grantReceiptAppName,
  grantReceiptClientId,
  grantReceiptOutcome,
  STEP_UP_STREAM_KEYS,
  stepUpLines,
} from '../src/grantHistory';

// The catalogues are read as data: check-i18n-keys.sh scans the clients' own
// t('…') literals only, so the keys these tables name for both clients are
// checked against every locale here.
const LOCALES: Record<string, unknown> = { de, en, es, fr, pt };

function lookup(catalogue: unknown, key: string): unknown {
  return key.split('.').reduce<unknown>(
    (node, part) => (typeof node === 'object' && node !== null ? (node as Record<string, unknown>)[part] : undefined),
    catalogue,
  );
}

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
    expect(grantReceiptOutcome(receipt('step_up', 'user'))).toBe('allowed');
    expect(grantReceiptOutcome(receipt('consume', 'client'))).toBe('endedByApp');
  });

  it('reads a payload without an approval member as allowed', () => {
    expect(grantReceiptOutcome({ ...receipt('issue', 'auto'), payload: {} })).toBe('allowed');
  });
});

describe('grantReceiptAppName', () => {
  it('names the app the server read, then a live grant or policy, then the client id', () => {
    const names = new Map([['https://client.example/meta.json', 'Live name']]);
    expect(grantReceiptAppName({ ...receipt('revoke', 'user'), client_name: 'Server name' }, names)).toBe('Server name');
    expect(grantReceiptAppName(receipt('revoke', 'user'), names)).toBe('Live name');
    expect(grantReceiptAppName({ ...receipt('revoke', 'user'), client_name: null }, new Map())).toBe(
      'https://client.example/meta.json',
    );
    expect(grantReceiptAppName({ ...receipt('issue', 'user'), payload: {} }, names)).toBeNull();
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

describe('the keys both clients name an outcome and a preset with', () => {
  it('are carried by every locale', () => {
    const keys = [...Object.values(GRANT_RECEIPT_OUTCOME_KEYS), ...Object.values(CONNECTED_APP_PRESET_KEYS)];
    for (const [locale, catalogue] of Object.entries(LOCALES)) {
      for (const key of keys) {
        expect(typeof lookup(catalogue, key), `${locale}: ${key}`).toBe('string');
      }
    }
  });

  it('names the known presets, and none for a plain grant, an unknown preset or an inherited name', () => {
    const t = (key: string) => `<${key}>`;
    expect(connectedAppPresetLabel('training_coach', t)).toBe('<tokens.connectedAppsPresetTrainingCoach>');
    expect(connectedAppPresetLabel('sleep_coach', t)).toBe('<tokens.connectedAppsPresetSleepCoach>');
    expect(connectedAppPresetLabel(null, t)).toBeNull();
    expect(connectedAppPresetLabel('recovery_coach', t)).toBeNull();
    expect(connectedAppPresetLabel('toString', t)).toBeNull();
  });
});

describe('stepUpLines', () => {
  const t = (key: string, options?: Record<string, unknown>) =>
    options ? `${key}(${JSON.stringify(options)})` : key;

  it('names the ticked streams, then the basis the app stands on', () => {
    expect(
      stepUpLines(
        { id: 'c', expires_at: null, streams: ['labs.summary', 'mystery.summary'], host_profile_basis: 'declared_request' },
        t,
      ),
    ).toEqual([
      'tokens.connectedAppsStepUpStreams({"streams":"oauth.stepUp.stream.labs, mystery.summary"})',
      'tokens.connectedAppsStepUpDeclaredRequest',
    ]);
    expect(stepUpLines({ id: 'c', expires_at: null, streams: [], host_profile_basis: 'declared_cimd' }, t)).toEqual([
      'tokens.connectedAppsStepUpDeclaredCimd',
    ]);
  });

  it('says nothing a server predating it did not send', () => {
    expect(stepUpLines({ id: 'c', expires_at: null }, t)).toEqual([]);
    expect(stepUpLines({ id: 'c', expires_at: null, streams: ['toString'] }, t)).toEqual([
      'tokens.connectedAppsStepUpStreams({"streams":"toString"})',
    ]);
  });

  it('labels each stream with a line every locale words', () => {
    for (const corpus of [en, fr, es, de, pt]) {
      const stream = (corpus as { oauth: { stepUp: { stream: Record<string, string> } } }).oauth.stepUp.stream;
      for (const key of Object.values(STEP_UP_STREAM_KEYS)) {
        expect(stream[key.replace('oauth.stepUp.stream.', '')], key).toBeTruthy();
      }
    }
  });
});
