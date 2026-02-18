// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for mobile usage status warning state computation
// ABOUTME: Validates warning levels, blocked messages, and severity prioritization

jest.mock('../src/services/api', () => ({
  apiClient: { get: jest.fn() },
}));

jest.mock('@pierre/shared-constants', () => ({
  QUERY_KEYS: { usage: { status: () => ['usage', 'status'] } },
}));

import { computeWarningState, type WarningLevel } from '../src/screens/chat/useUsageStatus';

interface LimitCheckResult {
  allowed: boolean;
  current: number;
  limit: number;
  warning: boolean;
  burst_zone: boolean;
  resets_at: string;
}

interface UsageStatusResponse {
  daily: {
    messages: LimitCheckResult;
    tokens: LimitCheckResult;
    tool_calls: LimitCheckResult;
  };
  weekly: {
    messages: LimitCheckResult;
    tokens: LimitCheckResult;
    tool_calls: LimitCheckResult;
  };
  resources: {
    conversations: number;
    max_conversations: number;
    coaches: number;
    max_coaches: number;
  };
}

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
      coaches: 3,
      max_coaches: 10,
    },
  };
}

describe('computeWarningState', () => {
  it('returns none when all counters are within limits', () => {
    const data = makeStatusResponse({});
    const state = computeWarningState(data);

    expect(state.level).toBe('none');
    expect(state.sendDisabled).toBe(false);
    expect(state.message).toBe('');
  });

  it('returns none when data is undefined', () => {
    const state = computeWarningState(undefined);

    expect(state.level).toBe('none');
    expect(state.sendDisabled).toBe(false);
    expect(state.message).toBe('');
    expect(state.resetsAt).toBe('');
  });

  it('returns warning when daily messages hit warning threshold', () => {
    const data = makeStatusResponse({
      dailyMessages: makeLimitCheck({ current: 40, limit: 50, warning: true }),
    });
    const state = computeWarningState(data);

    expect(state.level).toBe('warning');
    expect(state.sendDisabled).toBe(false);
    expect(state.message).toContain('80%');
    expect(state.message).toContain('daily messages');
  });

  it('returns burst when in burst zone', () => {
    const data = makeStatusResponse({
      dailyMessages: makeLimitCheck({ current: 55, limit: 50, warning: true, burst_zone: true }),
    });
    const state = computeWarningState(data);

    expect(state.level).toBe('burst');
    expect(state.sendDisabled).toBe(false);
    expect(state.message).toContain('burst zone');
    expect(state.message).toContain('daily messages');
  });

  it('returns blocked when not allowed', () => {
    const data = makeStatusResponse({
      dailyMessages: makeLimitCheck({ current: 75, limit: 50, allowed: false, warning: true, burst_zone: true }),
    });
    const state = computeWarningState(data);

    expect(state.level).toBe('blocked');
    expect(state.sendDisabled).toBe(true);
    expect(state.message).toContain('limit reached');
  });

  it('uses the triggering counter label in blocked message', () => {
    const data = makeStatusResponse({
      dailyTokens: makeLimitCheck({ current: 100, limit: 50, allowed: false, warning: true, burst_zone: true }),
    });
    const state = computeWarningState(data);

    expect(state.level).toBe('blocked');
    expect(state.message).toContain('Daily tokens');
  });

  it('picks the most severe level across counters', () => {
    const data = makeStatusResponse({
      dailyMessages: makeLimitCheck({ current: 40, limit: 50, warning: true }), // warning
      dailyTokens: makeLimitCheck({ current: 55, limit: 50, warning: true, burst_zone: true }), // burst
    });
    const state = computeWarningState(data);

    expect(state.level).toBe('burst');
    expect(state.message).toContain('daily tokens');
  });

  it('blocked overrides burst and warning', () => {
    const data = makeStatusResponse({
      dailyMessages: makeLimitCheck({ current: 40, limit: 50, warning: true }), // warning
      dailyTokens: makeLimitCheck({ current: 55, limit: 50, warning: true, burst_zone: true }), // burst
      weeklyMessages: makeLimitCheck({ current: 200, limit: 100, allowed: false }), // blocked
    });
    const state = computeWarningState(data);

    expect(state.level).toBe('blocked');
    expect(state.sendDisabled).toBe(true);
    expect(state.message).toContain('Weekly messages');
  });

  it('includes reset time in message', () => {
    const data = makeStatusResponse({
      dailyMessages: makeLimitCheck({ current: 40, limit: 50, warning: true, resets_at: '2026-02-18T12:00:00Z' }),
    });
    const state = computeWarningState(data);

    expect(state.message).toContain('reset');
    expect(state.resetsAt).toBe('2026-02-18T12:00:00Z');
  });
});
