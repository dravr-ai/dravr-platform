// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The settings provider row's way to the Intervals.icu API key from a card the athlete authorized
// ABOUTME: A coach's key reads their athletes and an authorization cannot, so the row offers it without a disconnect first (carnet#1820)

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { ProviderStatus } from '@pierre/shared-types';
import { i18n } from '@pierre/i18n';
import { ThemeProvider } from '../../../hooks/useTheme';
import ProviderRow from '../ProviderRow';

function card(overrides: Partial<ProviderStatus>): ProviderStatus {
  return {
    provider: 'intervals_icu',
    display_name: 'Intervals.icu',
    requires_oauth: true,
    connected: true,
    needs_reauth: false,
    capabilities: ['activities'],
    consent_required: false,
    ...overrides,
  } as ProviderStatus;
}

function renderRow(provider: ProviderStatus, onLinkKey = vi.fn()) {
  render(
    <ThemeProvider>
      <ProviderRow
        provider={provider}
        importsOverview={undefined}
        isConnecting={false}
        aiConsentPending={false}
        onConnect={vi.fn()}
        onAddHistory={vi.fn()}
        onDisconnect={vi.fn()}
        onLinkKey={onLinkKey}
        onWithdrawAiConsent={vi.fn()}
      />
    </ThemeProvider>,
  );
  return onLinkKey;
}

describe('ProviderRow — the Intervals.icu key', () => {
  beforeEach(async () => {
    await i18n.changeLanguage('en');
  });

  it('offers the key form on an authorized card, which had only Disconnect', async () => {
    const onLinkKey = renderRow(card({ connection_type: 'oauth' }));

    await userEvent.setup().click(screen.getByText(i18n.t('shell.intervalsCoachKeyAction')));

    expect(onLinkKey).toHaveBeenCalledTimes(1);
    expect(screen.getByTestId('provider-disconnect-intervals_icu')).toBeInTheDocument();
  });

  it('offers it on no key-linked card', () => {
    renderRow(card({ connection_type: 'manual' }));
    expect(screen.queryByTestId('provider-link-key-intervals_icu')).toBeNull();
  });

  it('offers it on no other provider', () => {
    renderRow(card({ provider: 'strava', display_name: 'Strava', connection_type: 'oauth' }));
    expect(screen.queryByTestId('provider-link-key-intervals_icu')).toBeNull();
  });
});
