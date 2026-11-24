// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The usage banner's sentences as the mobile catalogue prints them, through the real translator
// ABOUTME: Red if a counter's sentence goes back to a slotted name or to ungrouped figures

import type { LimitCheckResult, UsageStatusResponse } from '@pierre/shared-types';
import { i18n } from '@pierre/i18n';
import { computeWarningState } from '@pierre/chat-utils';

function makeLimitCheck(overrides: Partial<LimitCheckResult> = {}): LimitCheckResult {
  return {
    allowed: true,
    current: 0,
    limit: 50,
    warning: false,
    burst_zone: false,
    resets_at: '2026-02-18T00:00:00Z',
    ...overrides,
  };
}

function makeStatusResponse(overrides: Partial<{
  dailyMessages: LimitCheckResult;
  dailyTokens: LimitCheckResult;
}>): UsageStatusResponse {
  return {
    daily: {
      messages: overrides.dailyMessages ?? makeLimitCheck(),
      tokens: overrides.dailyTokens ?? makeLimitCheck(),
      tool_calls: makeLimitCheck(),
    },
    weekly: {
      messages: makeLimitCheck(),
      tokens: makeLimitCheck(),
      tool_calls: makeLimitCheck(),
    },
    resources: {
      conversations: 5,
      max_conversations: 20,
      agents: 3,
      max_agents: 10,
    },
  };
}

/** The sentence the banner shows: the shared decision's key and params through the real catalogue. */
function frenchSentence(data: UsageStatusResponse): string {
  const { text } = computeWarningState(data, i18n.t('settingsUi.midnightUtc'), 'fr');
  if (text === null) throw new Error('the counters set no banner');
  return i18n.t(text.key, text.params);
}

describe('the usage banner sentence in French', () => {
  beforeEach(async () => {
    await i18n.changeLanguage('fr');
  });
  afterEach(async () => {
    await i18n.changeLanguage('en');
  });

  it('writes a token cap grouped as a French athlete reads numbers', () => {
    // The sentence carried the raw counts, `(456792/500000)`, under French
    // chrome; the real translator now prints `(456 792/500 000)`.
    const message = frenchSentence(makeStatusResponse({
      dailyTokens: makeLimitCheck({ current: 456_792, limit: 500_000, warning: true }),
    }));

    expect(message).toContain('91 % de tes jetons quotidiens (456\u202f792/500\u202f000)');
    expect(message).not.toContain('456792');
  });

  it('says a reached daily message limit as one sentence', () => {
    // "Limite {{label}} atteinte" filled with "tes messages quotidiens" read
    // "Limite tes messages quotidiens atteinte".
    const message = frenchSentence(makeStatusResponse({
      dailyMessages: makeLimitCheck({ current: 50, limit: 50, allowed: false }),
    }));

    expect(message).toMatch(
      /^Limite de messages quotidiens atteinte\. Les limites se réinitialisent à .+\.$/,
    );
  });
});
