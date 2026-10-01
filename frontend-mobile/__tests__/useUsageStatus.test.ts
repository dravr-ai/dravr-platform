// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for mobile usage status warning state computation
// ABOUTME: Validates warning levels, blocked messages, and severity prioritization

jest.mock('../src/services/api', () => ({
  usageApi: { getStatus: jest.fn() },
}));

jest.mock('@pierre/shared-constants', () => ({
  QUERY_KEYS: { usage: { status: () => ['usage', 'status'] } },
}));

import type { LimitCheckResult, UsageStatusResponse } from '@pierre/shared-types';
import { i18n } from '@pierre/i18n';
import { computeWarningState, type WarningLevel } from '../src/screens/chat/useUsageStatus';

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
  weeklyMessages: LimitCheckResult;
}>): UsageStatusResponse {
  return {
    daily: {
      messages: overrides.dailyMessages ?? makeLimitCheck(),
      tokens: overrides.dailyTokens ?? makeLimitCheck(),
      tool_calls: makeLimitCheck(),
    },
    weekly: {
      messages: overrides.weeklyMessages ?? makeLimitCheck(),
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

/**
 * A translator that returns the key and appends its params, so an assertion
 * names the key the hook chose without pinning any locale's wording.
 */
const translate = (key: string, params?: Record<string, string | number>): string =>
  params === undefined ? key : `${key} ${JSON.stringify(params)}`;

describe('computeWarningState', () => {
  it('returns none when all counters are within limits', () => {
    const data = makeStatusResponse({});
    const state = computeWarningState(data, translate, 'en');

    expect(state.level).toBe('none');
    expect(state.sendDisabled).toBe(false);
    expect(state.message).toBe('');
  });

  it('returns none when data is undefined', () => {
    const state = computeWarningState(undefined, translate, 'en');

    expect(state.level).toBe('none');
    expect(state.sendDisabled).toBe(false);
    expect(state.message).toBe('');
    expect(state.resetsAt).toBe('');
  });

  it('returns warning when daily messages hit warning threshold', () => {
    const data = makeStatusResponse({
      dailyMessages: makeLimitCheck({ current: 40, limit: 50, warning: true }),
    });
    const state = computeWarningState(data, translate, 'en');

    expect(state.level).toBe('warning');
    expect(state.sendDisabled).toBe(false);
    expect(state.message).toContain('"percent":80');
    expect(state.message).toContain('usage.used.dailyMessages');
  });

  it('returns burst when in burst zone', () => {
    const data = makeStatusResponse({
      dailyMessages: makeLimitCheck({ current: 55, limit: 50, warning: true, burst_zone: true }),
    });
    const state = computeWarningState(data, translate, 'en');

    expect(state.level).toBe('burst');
    expect(state.sendDisabled).toBe(false);
    expect(state.message).toContain('usage.burst.dailyMessages');
  });

  it('returns blocked when not allowed', () => {
    const data = makeStatusResponse({
      dailyMessages: makeLimitCheck({ current: 75, limit: 50, allowed: false, warning: true, burst_zone: true }),
    });
    const state = computeWarningState(data, translate, 'en');

    expect(state.level).toBe('blocked');
    expect(state.sendDisabled).toBe(true);
    expect(state.message).toContain('usage.reached.dailyMessages');
  });

  it("uses the triggering counter's own sentence when blocked", () => {
    const data = makeStatusResponse({
      dailyTokens: makeLimitCheck({ current: 100, limit: 50, allowed: false, warning: true, burst_zone: true }),
    });
    const state = computeWarningState(data, translate, 'en');

    expect(state.level).toBe('blocked');
    expect(state.message).toContain('usage.reached.dailyTokens');
  });

  it('picks the most severe level across counters', () => {
    const data = makeStatusResponse({
      dailyMessages: makeLimitCheck({ current: 40, limit: 50, warning: true }), // warning
      dailyTokens: makeLimitCheck({ current: 55, limit: 50, warning: true, burst_zone: true }), // burst
    });
    const state = computeWarningState(data, translate, 'en');

    expect(state.level).toBe('burst');
    expect(state.message).toContain('usage.burst.dailyTokens');
  });

  it('blocked overrides burst and warning', () => {
    const data = makeStatusResponse({
      dailyMessages: makeLimitCheck({ current: 40, limit: 50, warning: true }), // warning
      dailyTokens: makeLimitCheck({ current: 55, limit: 50, warning: true, burst_zone: true }), // burst
      weeklyMessages: makeLimitCheck({ current: 200, limit: 100, allowed: false }), // blocked
    });
    const state = computeWarningState(data, translate, 'en');

    expect(state.level).toBe('blocked');
    expect(state.sendDisabled).toBe(true);
    expect(state.message).toContain('usage.reached.weeklyMessages');
  });

  it('includes reset time in message', () => {
    const data = makeStatusResponse({
      dailyMessages: makeLimitCheck({ current: 40, limit: 50, warning: true, resets_at: '2026-02-18T12:00:00Z' }),
    });
    const state = computeWarningState(data, translate, 'en');

    expect(state.message).toContain('"time"');
    expect(state.resetsAt).toBe('2026-02-18T12:00:00Z');
  });

  it('writes a token cap in French, grouped as a French athlete reads numbers', async () => {
    // The sentence carried the raw counts, `(456792/500000)`, under French
    // chrome; the real translator now prints `(456 792/500 000)`.
    await i18n.changeLanguage('fr');
    try {
      const data = makeStatusResponse({
        dailyTokens: makeLimitCheck({ current: 456_792, limit: 500_000, warning: true }),
      });
      const state = computeWarningState(data, i18n.t.bind(i18n), 'fr');

      expect(state.message).toContain('91 % de tes jetons quotidiens (456\u202f792/500\u202f000)');
      expect(state.message).not.toContain('456792');
    } finally {
      await i18n.changeLanguage('en');
    }
  });

  it('says a reached daily message limit as one French sentence', async () => {
    // "Limite {{label}} atteinte" filled with "tes messages quotidiens" read
    // "Limite tes messages quotidiens atteinte".
    await i18n.changeLanguage('fr');
    try {
      const data = makeStatusResponse({
        dailyMessages: makeLimitCheck({ current: 50, limit: 50, allowed: false }),
      });
      const state = computeWarningState(data, i18n.t.bind(i18n), 'fr');

      expect(state.message).toMatch(
        /^Limite de messages quotidiens atteinte\. Les limites se réinitialisent à .+\.$/,
      );
    } finally {
      await i18n.changeLanguage('en');
    }
  });
});
