// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the provider banner — the shell's reconnect strip names every connected provider that needs reauth
// ABOUTME: Red if a healthy or disconnected provider raises the strip, a name repeats, or its action misses the connections pane

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ExtendedProviderStatus, ProvidersStatusResponse } from '@pierre/shared-types';
import { ConnectProviderBanner, type ProviderBannerKind } from '../ConnectProviderBanner';
import { providerStatus } from '../home/__tests__/homeFixtures';

const api = vi.hoisted(() => ({
  getProvidersStatus: vi.fn<() => Promise<ProvidersStatusResponse>>(),
}));

vi.mock('../../services/api', () => ({
  providersApi: { getProvidersStatus: api.getProvidersStatus },
}));

function providers(...list: ExtendedProviderStatus[]) {
  api.getProvidersStatus.mockResolvedValue({ providers: list });
}

function renderBanner(kind: ProviderBannerKind) {
  const onNavigate = vi.fn();
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={queryClient}>
      <ConnectProviderBanner kind={kind} onNavigate={onNavigate} />
    </QueryClientProvider>,
  );
  /** The provider-status read has answered — what an absence has to wait for. */
  const settled = () => waitFor(() => expect(queryClient.isFetching()).toBe(0));
  return { onNavigate, settled };
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe('ConnectProviderBanner — reconnect', () => {
  it('names the provider to reconnect in an alert and leads to the connections pane', async () => {
    providers(
      providerStatus({ provider: 'strava', display_name: 'Strava' }),
      providerStatus({ provider: 'garmin', display_name: 'Garmin', needs_reauth: true }),
    );
    const { onNavigate } = renderBanner('reconnect');

    const banner = await screen.findByRole('alert');
    expect(banner).toHaveAttribute('data-testid', 'provider-reconnect-banner');
    expect(banner).toHaveTextContent('Reconnect needed');
    expect(banner).toHaveTextContent('Reconnect Garmin to see your new activities.');
    expect(banner).not.toHaveTextContent('Strava');
    // Critical, so there is nothing to dismiss it with.
    expect(within(banner).getAllByRole('button')).toHaveLength(1);

    await userEvent.click(within(banner).getByRole('button', { name: 'Reconnect' }));
    expect(onNavigate).toHaveBeenCalledExactlyOnceWith('settings/connections');
  });

  it('names every provider once, in server order, when a mirror lists one twice', async () => {
    providers(
      providerStatus({ provider: 'garmin', display_name: 'Garmin', needs_reauth: true }),
      providerStatus({ provider: 'garmin_sciotte', display_name: 'Garmin', needs_reauth: true }),
      providerStatus({ provider: 'coros', display_name: 'COROS', needs_reauth: true }),
    );
    renderBanner('reconnect');

    expect(await screen.findByTestId('provider-reconnect-banner')).toHaveTextContent(
      'Reconnect Garmin and COROS to see your new activities.',
    );
  });

  it('shows nothing while every connected provider is healthy', async () => {
    providers(
      providerStatus({ provider: 'strava', display_name: 'Strava' }),
      providerStatus({ provider: 'garmin', display_name: 'Garmin' }),
    );
    const { settled } = renderBanner('reconnect');

    await settled();
    expect(api.getProvidersStatus).toHaveBeenCalledTimes(1);
    expect(screen.queryByTestId('provider-reconnect-banner')).toBeNull();
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('shows nothing for a provider flagged needs_reauth that is not connected', async () => {
    providers(
      providerStatus({ provider: 'strava', display_name: 'Strava' }),
      providerStatus({ provider: 'garmin', display_name: 'Garmin', connected: false, needs_reauth: true }),
    );
    const { settled } = renderBanner('reconnect');

    await settled();
    expect(api.getProvidersStatus).toHaveBeenCalledTimes(1);
    expect(screen.queryByTestId('provider-reconnect-banner')).toBeNull();
  });

  it('never shows the connect nudge from the shell mount, even with nothing connected', async () => {
    providers(providerStatus({ provider: 'strava', display_name: 'Strava', connected: false }));
    const { settled } = renderBanner('reconnect');

    await settled();
    expect(screen.queryByTestId('connect-provider-banner')).toBeNull();
    expect(screen.queryByTestId('provider-reconnect-banner')).toBeNull();
  });
});

describe('ConnectProviderBanner — connect', () => {
  it('nudges an athlete with no provider, leads to the connections pane, and can be dismissed', async () => {
    providers(providerStatus({ provider: 'strava', display_name: 'Strava', connected: false }));
    const { onNavigate } = renderBanner('connect');

    const nudge = await screen.findByTestId('connect-provider-banner');
    expect(nudge).toHaveTextContent('Connect a fitness provider');
    await userEvent.click(within(nudge).getByRole('button', { name: 'Connect' }));
    expect(onNavigate).toHaveBeenCalledExactlyOnceWith('settings/connections');

    await userEvent.click(within(nudge).getByRole('button', { name: 'Dismiss' }));
    expect(screen.queryByTestId('connect-provider-banner')).toBeNull();
  });

  it('stays quiet for a connected athlete, including one whose provider needs reconnecting', async () => {
    providers(providerStatus({ provider: 'garmin', display_name: 'Garmin', needs_reauth: true }));
    const { settled } = renderBanner('connect');

    await settled();
    expect(screen.queryByTestId('connect-provider-banner')).toBeNull();
    expect(screen.queryByTestId('provider-reconnect-banner')).toBeNull();
  });
});
