// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for usage warning state computation and banner rendering
// ABOUTME: Validates warning levels, dismiss behavior, and send blocking

import { describe, it, expect } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import UsageWarningBanner from '../chat/UsageWarningBanner';
import { computeWarningState } from '../../hooks/useUsageStatus';
import type { UsageStatusResponse } from '../../services/api/usage';

function makeLimitCheck(overrides: Partial<{
  allowed: boolean;
  current: number;
  limit: number;
  warning: boolean;
  burst_zone: boolean;
  resets_at: string;
}> = {}) {
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
  dailyMessages: ReturnType<typeof makeLimitCheck>;
  dailyTokens: ReturnType<typeof makeLimitCheck>;
  weeklyMessages: ReturnType<typeof makeLimitCheck>;
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

  it('picks the most severe level across counters', () => {
    const data = makeStatusResponse({
      dailyMessages: makeLimitCheck({ current: 40, limit: 50, warning: true }), // warning
      dailyTokens: makeLimitCheck({ current: 55, limit: 50, warning: true, burst_zone: true }), // burst
    });
    const state = computeWarningState(data);

    expect(state.level).toBe('burst');
  });
});

describe('UsageWarningBanner', () => {
  it('renders nothing when level is none', () => {
    const { container } = render(<UsageWarningBanner level="none" message="" />);
    expect(container.firstChild).toBeNull();
  });

  it('renders warning banner with yellow styling', () => {
    render(<UsageWarningBanner level="warning" message="80% of daily messages used" />);
    const banner = screen.getByTestId('usage-warning-banner');
    expect(banner).toBeDefined();
    expect(banner.textContent).toContain('80% of daily messages used');
    expect(banner.className).toContain('yellow');
  });

  it('renders burst banner with orange styling', () => {
    render(<UsageWarningBanner level="burst" message="In burst zone" />);
    const banner = screen.getByTestId('usage-warning-banner');
    expect(banner.className).toContain('orange');
  });

  it('renders blocked banner with red styling', () => {
    render(<UsageWarningBanner level="blocked" message="Limit reached" />);
    const banner = screen.getByTestId('usage-warning-banner');
    expect(banner.className).toContain('red');
  });

  it('can be dismissed when not blocked', () => {
    render(<UsageWarningBanner level="warning" message="80% used" />);
    const dismissBtn = screen.getByLabelText('Dismiss warning');
    fireEvent.click(dismissBtn);
    expect(screen.queryByTestId('usage-warning-banner')).toBeNull();
  });

  it('cannot be dismissed when blocked', () => {
    render(<UsageWarningBanner level="blocked" message="Limit reached" />);
    expect(screen.queryByLabelText('Dismiss warning')).toBeNull();
    expect(screen.getByTestId('usage-warning-banner')).toBeDefined();
  });
});
