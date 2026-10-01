// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves every way a chat turn fails reads as a catalogue sentence, keyed on the server's code
// ABOUTME: The server's English description is never what the athlete reads (carnet#680)

import { describe, it, expect } from 'vitest';
import { TurnFailedError, TurnIdleAbortedError, TurnRequestError } from '@pierre/api-client';
import i18next from 'i18next';
import fr from '../../i18n/src/locales/fr/translation.json';
import { describeTurnFailure, isTurnFailureRetryable } from '../src/turnFailure';

/**
 * The real French catalogue through a real i18next, so a slot's format —
 * `{{current, number}}` groups 500000 as 500 000 — is applied exactly as the
 * clients apply it, rather than by a hand-rolled `{{name}}` replace.
 */
const french = i18next.createInstance();
void french.init({
  lng: 'fr',
  resources: { fr: { translation: fr } },
  initAsync: false,
  showSupportNotice: false,
  interpolation: { escapeValue: false },
});

function frenchT(key: string, params?: Record<string, string | number>): string {
  if (!french.exists(key)) throw new Error(`missing catalogue key ${key}`);
  return french.t(key, params ?? {});
}

const ENGLISH = 'The resource is temporarily unavailable';

describe('describeTurnFailure', () => {
  it('words a fail-fast 503 from its code, in French, never the server text', () => {
    const refused = new TurnRequestError(ENGLISH, 503, { code: 'ResourceUnavailable', message: ENGLISH });
    const text = describeTurnFailure(refused, { t: frenchT });
    expect(text).toBe(fr.errors.generic);
    expect(text).not.toContain('temporarily');
  });

  it('words a mid-stream failure from the failed frame code', () => {
    const failed = new TurnFailedError(ENGLISH, 'ResourceUnavailable');
    expect(describeTurnFailure(failed, { t: frenchT })).toBe(fr.errors.generic);
  });

  it('reads an internal failure as a server error, not its description', () => {
    const failed = new TurnFailedError('Internal server error', 'InternalError');
    expect(describeTurnFailure(failed, { t: frenchT })).toBe(fr.errors.serverError);
  });

  it('words a refusal whose code it has no sentence for by its status, not as a server fault', () => {
    const gated = new TurnRequestError('Connect a provider first', 403, {
      code: 'NoProviderConnected',
      message: 'Connect a provider first',
    });
    expect(describeTurnFailure(gated, { t: frenchT })).toBe(fr.errors.forbidden);
  });

  it('reads a turn that ended with no reason as a server error', () => {
    const broken = new TurnFailedError('The turn ended without a reply.', null);
    expect(describeTurnFailure(broken, { t: frenchT })).toBe(fr.errors.serverError);
  });

  it('names a spent quota by its own counts, grouped the French way', () => {
    const failed = new TurnFailedError('Daily message limit reached', 'QuotaExceeded', {
      limit_type: 'daily_messages',
      current: 1500,
      limit: 1500,
    });
    expect(describeTurnFailure(failed, { t: frenchT })).toBe(
      'Limite de messages quotidiens atteinte (1\u202f500/1\u202f500). Réinitialisation demain.',
    );
  });

  it('names the archived-thread cap apart from the new-conversation cap, on either side of the stream', () => {
    const details = {
      limit_type: 'max_active_conversations',
      current: 10,
      limit: 10,
      reason: 'conversation_archived',
    };
    const expected = frenchT('errors.archivedConversationLimitReached', { current: 10, limit: 10 });
    expect(expected).not.toBe(frenchT('errors.conversationLimitReached', { current: 10, limit: 10 }));
    const midStream = new TurnFailedError('quota exceeded', 'QuotaExceeded', details, 429);
    expect(describeTurnFailure(midStream, { t: frenchT })).toBe(expected);
    const refused = new TurnRequestError('quota exceeded', 429, {
      code: 'QuotaExceeded',
      message: 'quota exceeded',
      details,
    });
    expect(describeTurnFailure(refused, { t: frenchT })).toBe(expected);
    expect(isTurnFailureRetryable(midStream)).toBe(false);
  });

  it('keeps the idle stop its own guidance', () => {
    expect(describeTurnFailure(new TurnIdleAbortedError(), { t: frenchT })).toBe(fr.chat.turnIdleAborted);
  });

  it('reads a request that never landed as the network, or offline', () => {
    const lost = new TypeError('Network request failed');
    expect(describeTurnFailure(lost, { t: frenchT })).toBe(fr.errors.network);
    expect(describeTurnFailure(lost, { t: frenchT, online: false })).toBe(fr.errors.offline);
  });

  it('reads an uncoded 503 as the same outage', () => {
    const refused = new TurnRequestError('The server answered 503.', 503, null);
    expect(describeTurnFailure(refused, { t: frenchT })).toBe(fr.errors.generic);
  });
});

describe('isTurnFailureRetryable', () => {
  it('offers a retry for a transient code, refused or mid-stream', () => {
    for (const code of ['ResourceUnavailable', 'ExternalServiceUnavailable', 'ExternalServiceError', 'ExternalRateLimited', 'RateLimitExceeded']) {
      expect(isTurnFailureRetryable(new TurnFailedError(ENGLISH, code)), code).toBe(true);
      expect(isTurnFailureRetryable(new TurnRequestError(ENGLISH, 503, { code })), code).toBe(true);
    }
  });

  it('offers none for a refusal a retry would only repeat', () => {
    for (const code of ['QuotaExceeded', 'PermissionDenied', 'AuthRequired', 'AuthExpired', 'InvalidInput', 'InternalError']) {
      expect(isTurnFailureRetryable(new TurnFailedError(ENGLISH, code)), code).toBe(false);
      expect(isTurnFailureRetryable(new TurnRequestError(ENGLISH, 400, { code })), code).toBe(false);
    }
    expect(isTurnFailureRetryable(new TurnFailedError('The turn ended without a reply.', null))).toBe(false);
  });

  it('offers one for an uncoded 503 and a request that never arrived, none for the idle stop', () => {
    expect(isTurnFailureRetryable(new TurnRequestError(ENGLISH, 503, {}))).toBe(true);
    expect(isTurnFailureRetryable(new TypeError('Failed to fetch'), { online: false })).toBe(true);
    expect(isTurnFailureRetryable(new TypeError('Failed to fetch'))).toBe(true);
    expect(isTurnFailureRetryable(new TurnRequestError(ENGLISH, 429, {}))).toBe(false);
    expect(isTurnFailureRetryable(new TurnIdleAbortedError())).toBe(false);
  });
});
