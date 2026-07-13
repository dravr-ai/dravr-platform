// ABOUTME: Unit tests for ProviderConnectionCards OAuth-first-with-Sciotte-fallback behavior
// ABOUTME: Covers the seat-gated OAuth default, the mirror path, and the failed-OAuth fallback listener
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor, act } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import ProviderConnectionCards from '../ProviderConnectionCards';

const getAuthorizeUrlForProvider = vi.fn().mockResolvedValue('https://www.strava.com/oauth/authorize?x=1');
const getProvidersStatus = vi.fn();

vi.mock('../../services/api', () => ({
  providersApi: { getProvidersStatus: (...args: unknown[]) => getProvidersStatus(...args) },
  oauthApi: { getAuthorizeUrlForProvider: (...args: unknown[]) => getAuthorizeUrlForProvider(...args) },
}));

vi.mock('../../services/analytics', () => ({ track: vi.fn() }));

// Render the Sciotte modal as a testid carrying its target so a fallback that
// opens it (target="strava") is observable.
vi.mock('../SciotteLoginModal', () => ({
  default: ({ isOpen, target }: { isOpen: boolean; target: string }) =>
    isOpen ? <div data-testid="sciotte-modal">{target}</div> : null,
}));
vi.mock('../IntervalsIcuLinkModal', () => ({
  default: () => null,
}));

// The `sciotte` card IS the user-facing "Strava" card; display_name comes from
// the server as "Strava".
function stravaCard(recommended_backend: 'oauth' | 'mirror') {
  return {
    provider: 'sciotte',
    display_name: 'Strava',
    requires_oauth: false,
    connected: false,
    capabilities: ['activities'],
    recommended_backend,
    seats_left: recommended_backend === 'oauth' ? 3 : 0,
  };
}

function renderCards() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <ProviderConnectionCards />
    </QueryClientProvider>,
  );
}

describe('ProviderConnectionCards — OAuth-first with Sciotte fallback', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    // window.open is used to pre-open the OAuth popup; return a live fake.
    vi.stubGlobal('open', vi.fn().mockReturnValue({ closed: false, location: { href: '' } }));
  });

  it('launches Strava OAuth (not the Sciotte modal) while seats remain', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [stravaCard('oauth')] });
    const user = userEvent.setup();
    renderCards();

    const button = await screen.findByLabelText('Connect to Strava');
    await user.click(button);

    await waitFor(() => expect(getAuthorizeUrlForProvider).toHaveBeenCalledWith('strava'));
    expect(screen.queryByTestId('sciotte-modal')).not.toBeInTheDocument();
  });

  it('opens the Sciotte modal directly (no OAuth) once the pool is exhausted', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [stravaCard('mirror')] });
    const user = userEvent.setup();
    renderCards();

    const button = await screen.findByLabelText('Connect to Strava');
    await user.click(button);

    expect(await screen.findByTestId('sciotte-modal')).toHaveTextContent('strava');
    expect(getAuthorizeUrlForProvider).not.toHaveBeenCalled();
  });

  it('falls back to the Sciotte modal when a Strava OAuth attempt fails', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [stravaCard('oauth')] });
    renderCards();
    await screen.findByLabelText('Connect to Strava');

    // Simulate the OAuth callback tab reporting a failed Strava connection.
    const failed = JSON.stringify({
      type: 'oauth_completed',
      provider: 'strava',
      success: false,
      timestamp: Date.now(),
    });
    await act(async () => {
      localStorage.setItem('pierre_oauth_result', failed);
      window.dispatchEvent(new StorageEvent('storage', { key: 'pierre_oauth_result', newValue: failed }));
    });

    expect(await screen.findByTestId('sciotte-modal')).toHaveTextContent('strava');
    // The failed result is consumed so other observers don't double-handle it.
    expect(localStorage.getItem('pierre_oauth_result')).toBeNull();
  });

  it('does NOT consume a successful OAuth result (leaves it for the success handlers)', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [stravaCard('oauth')] });
    renderCards();
    await screen.findByLabelText('Connect to Strava');

    const ok = JSON.stringify({
      type: 'oauth_completed',
      provider: 'strava',
      success: true,
      timestamp: Date.now(),
    });
    await act(async () => {
      localStorage.setItem('pierre_oauth_result', ok);
      window.dispatchEvent(new StorageEvent('storage', { key: 'pierre_oauth_result', newValue: ok }));
    });

    expect(screen.queryByTestId('sciotte-modal')).not.toBeInTheDocument();
    expect(localStorage.getItem('pierre_oauth_result')).toBe(ok);
  });
});
