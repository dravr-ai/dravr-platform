// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for usage warning state computation and banner rendering
// ABOUTME: Validates warning levels, dismiss behavior, and send blocking

import { describe, it, expect } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import UsageWarningBanner from '../chat/UsageWarningBanner';
import { computeWarningState } from '../../hooks/useUsageStatus';
import type { UsageStatusResponse } from '@pierre/shared-types';
import { i18n } from '@pierre/i18n';

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
      agents: 3,
      max_agents: 10,
    },
  };
}

/** Params for the sentence under test. */
const SAMPLE_PARAMS = {
  current: 80,
  limit: 100,
  percent: 80,
  time: '9:00 AM',
};

describe('computeWarningState', () => {
  it('returns none when all counters are within limits', () => {
    const data = makeStatusResponse({});
    const state = computeWarningState(data, 'midnight UTC', 'en');

    expect(state.level).toBe('none');
    expect(state.sendDisabled).toBe(false);
    expect(state.text).toBeNull();
  });

  it('returns none when data is undefined', () => {
    const state = computeWarningState(undefined, 'midnight UTC', 'en');
    expect(state.level).toBe('none');
    expect(state.sendDisabled).toBe(false);
  });

  it('returns warning when daily messages hit warning threshold', () => {
    const data = makeStatusResponse({
      dailyMessages: makeLimitCheck({ current: 40, limit: 50, warning: true }),
    });
    const state = computeWarningState(data, 'midnight UTC', 'en');

    expect(state.level).toBe('warning');
    expect(state.sendDisabled).toBe(false);
    expect(state.text?.params?.percent).toBe(80);
    expect(state.text?.key).toBe('usage.used.dailyMessages');
  });

  it('returns burst when in burst zone', () => {
    const data = makeStatusResponse({
      dailyMessages: makeLimitCheck({ current: 55, limit: 50, warning: true, burst_zone: true }),
    });
    const state = computeWarningState(data, 'midnight UTC', 'en');

    expect(state.level).toBe('burst');
    expect(state.sendDisabled).toBe(false);
    expect(state.text?.key).toBe('usage.burst.dailyMessages');
  });

  it('returns blocked when not allowed', () => {
    const data = makeStatusResponse({
      dailyMessages: makeLimitCheck({ current: 75, limit: 50, allowed: false, warning: true, burst_zone: true }),
    });
    const state = computeWarningState(data, 'midnight UTC', 'en');

    expect(state.level).toBe('blocked');
    expect(state.sendDisabled).toBe(true);
    expect(state.text?.key).toBe('usage.reached.dailyMessages');
  });

  it('picks the most severe level across counters', () => {
    const data = makeStatusResponse({
      dailyMessages: makeLimitCheck({ current: 40, limit: 50, warning: true }), // warning
      dailyTokens: makeLimitCheck({ current: 55, limit: 50, warning: true, burst_zone: true }), // burst
    });
    const state = computeWarningState(data, 'midnight UTC', 'en');

    expect(state.level).toBe('burst');
  });
});

describe('UsageWarningBanner', () => {
  it('renders nothing when level is none', () => {
    const { container } = render(<UsageWarningBanner level="none" text={null} />);
    expect(container.firstChild).toBeNull();
  });

  it('renders warning banner on the warning token', () => {
    render(<UsageWarningBanner level="warning" text={{ key: 'usage.used.dailyMessages', params: SAMPLE_PARAMS }} />);
    const banner = screen.getByTestId('usage-warning-banner');
    expect(banner).toBeDefined();
    // The banner renders the counter's own catalogue sentence.
    expect(banner.textContent).toContain('80%');
    expect(banner.textContent).toContain('your daily messages');
    expect(banner.textContent).toContain('(80/100)');
    expect(banner.className).toContain('bg-warning/10');
  });

  it('renders burst banner on the warning token at heavier weight', () => {
    render(<UsageWarningBanner level="burst" text={{ key: 'usage.used.dailyMessages', params: SAMPLE_PARAMS }} />);
    const banner = screen.getByTestId('usage-warning-banner');
    expect(banner.className).toContain('bg-warning/25');
  });

  it('renders blocked banner on the error token', () => {
    render(<UsageWarningBanner level="blocked" text={{ key: 'usage.used.dailyMessages', params: SAMPLE_PARAMS }} />);
    const banner = screen.getByTestId('usage-warning-banner');
    expect(banner.className).toContain('bg-error/10');
  });

  // The three levels form an escalation ladder. Collapsing any two into the
  // same styling makes the escalation invisible to the user, which no
  // individual per-level assertion above would catch.
  it('styles every escalation level distinguishably', () => {
    const seen = new Set<string>();
    for (const level of ['warning', 'burst', 'blocked'] as const) {
      const { unmount } = render(<UsageWarningBanner level={level} text={{ key: 'usage.used.dailyMessages', params: SAMPLE_PARAMS }} />);
      seen.add(screen.getByTestId('usage-warning-banner').className);
      unmount();
    }
    expect(seen.size).toBe(3);
  });

  it('can be dismissed when not blocked', () => {
    render(<UsageWarningBanner level="warning" text={{ key: 'usage.used.dailyMessages', params: SAMPLE_PARAMS }} />);
    const dismissBtn = screen.getByLabelText('Dismiss warning');
    fireEvent.click(dismissBtn);
    expect(screen.queryByTestId('usage-warning-banner')).toBeNull();
  });

  it('prints a token cap grouped as a French athlete reads it', async () => {
    // The sentence carried the raw counts, `(456792/500000)`, under French
    // chrome; it reads `(456 792/500 000)`, in a French sentence.
    await i18n.changeLanguage('fr');
    try {
      const data = makeStatusResponse({
        dailyTokens: makeLimitCheck({ warning: true, current: 456_792, limit: 500_000 }),
      });
      const state = computeWarningState(data, 'minuit UTC', 'fr');
      render(<UsageWarningBanner level={state.level} text={state.text} />);

      const banner = screen.getByText(/de tes jetons quotidiens/);
      expect(banner.textContent).toContain('91 % de tes jetons quotidiens (456\u202f792/500\u202f000)');
      expect(banner.textContent).not.toContain('456792');
    } finally {
      await i18n.changeLanguage('en');
    }
  });

  it('says a reached limit as one sentence per counter, in French and English', async () => {
    // "Limite {{label}} atteinte" filled with "tes messages quotidiens" read
    // "Limite tes messages quotidiens atteinte", and English opened lowercase:
    // "your daily messages limit reached".
    const data = makeStatusResponse({
      dailyMessages: makeLimitCheck({ current: 50, limit: 50, allowed: false }),
    });
    await i18n.changeLanguage('fr');
    try {
      const fr = computeWarningState(data, 'minuit UTC', 'fr');
      const { unmount } = render(<UsageWarningBanner level={fr.level} text={fr.text} />);
      expect(screen.getByTestId('usage-warning-banner').textContent).toBe(
        `Limite de messages quotidiens atteinte. Les limites se réinitialisent à ${String(fr.text?.params?.time)}.`,
      );
      unmount();
    } finally {
      await i18n.changeLanguage('en');
    }
    const en = computeWarningState(data, 'midnight UTC', 'en');
    render(<UsageWarningBanner level={en.level} text={en.text} />);
    expect(screen.getByTestId('usage-warning-banner').textContent).toBe(
      `Daily message limit reached. Limits reset at ${String(en.text?.params?.time)}.`,
    );
  });

  it('inflects the counter it names in German', async () => {
    // "von {{label}}" took "deine täglichen Tokens" in the accusative.
    await i18n.changeLanguage('de');
    try {
      const data = makeStatusResponse({
        dailyTokens: makeLimitCheck({ warning: true, current: 456_792, limit: 500_000 }),
      });
      const state = computeWarningState(data, 'Mitternacht UTC', 'de');
      render(<UsageWarningBanner level={state.level} text={state.text} />);
      expect(screen.getByTestId('usage-warning-banner').textContent).toContain(
        'Du hast 91 % deiner täglichen Tokens (456.792/500.000) verbraucht.',
      );
    } finally {
      await i18n.changeLanguage('en');
    }
  });

  it('cannot be dismissed when blocked', () => {
    render(<UsageWarningBanner level="blocked" text={{ key: 'usage.used.dailyMessages', params: SAMPLE_PARAMS }} />);
    expect(screen.queryByLabelText('Dismiss warning')).toBeNull();
    expect(screen.getByTestId('usage-warning-banner')).toBeDefined();
  });
});
