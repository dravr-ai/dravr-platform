// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A refused turn's quota sentence writes its counts in the app language, through the app's own i18next
// ABOUTME: Guards the catalogue's {{current, number}} slots and the built-in formatter the web init leaves on

import { describe, it, expect } from 'vitest';
import { TurnFailedError } from '@pierre/api-client';
import { i18n } from '@pierre/i18n';
import { describeQuotaRefusal, describeTurnFailure } from '@pierre/ui-logic';

/** What a 429 at the daily token cap carries, on the HTTP refusal path. */
const refusal = {
  response: {
    status: 429,
    data: {
      code: 'QuotaExceeded',
      message: 'daily_tokens quota exceeded: 500000/500000',
      details: { limit_type: 'daily_tokens', current: 500000, limit: 500000 },
    },
  },
};

function translator(lng: string) {
  return (key: string, params?: Record<string, string | number>): string =>
    i18n.t(key, { ...params, lng });
}

describe('quota refusal counts', () => {
  it('groups thousands the French way — a narrow no-break space, not digits pasted raw', () => {
    expect(describeQuotaRefusal(refusal, translator('fr'))).toBe(
      'Limite de jetons quotidiens atteinte (500 000/500 000). Réinitialisation demain.',
    );
  });

  it('groups thousands the English way for an English athlete', () => {
    expect(describeQuotaRefusal(refusal, translator('en'))).toBe(
      'Daily token limit reached (500,000/500,000). Resets tomorrow.',
    );
  });

  it('formats the counts of a turn refused mid-stream the same way', () => {
    const failed = new TurnFailedError(
      'daily_tokens quota exceeded: 500000/500000',
      'QuotaExceeded',
      { limit_type: 'daily_tokens', current: 500000, limit: 500000 },
      429,
    );

    expect(describeTurnFailure(failed, { t: translator('de') })).toContain('(500.000/500.000)');
  });
});
