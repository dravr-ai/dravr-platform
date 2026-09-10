// ABOUTME: Unit tests for ProviderConnectionCards OAuth-first-with-Sciotte-fallback behavior
// ABOUTME: Covers the seat-gated OAuth default, the mirror path, and the failed-OAuth fallback listener
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen, waitFor, act } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { PROVIDER_LINK_POLL_INTERVAL_MS } from '@pierre/shared-constants';
import ProviderConnectionCards from '../ProviderConnectionCards';

const authorizeUrl = vi.fn((provider: string) => `/api/oauth/authorize/${provider}`);
const getProvidersStatus = vi.fn();

vi.mock('../../services/api', () => ({
  providersApi: { getProvidersStatus: (...args: unknown[]) => getProvidersStatus(...args) },
  oauthApi: { authorizeUrl: (...args: unknown[]) => authorizeUrl(...args) },
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
    needs_reauth: false,
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
    // The card opens the server's launch route directly; return a live fake so
    // the "popup blocked" same-tab fallback is not taken.
    vi.stubGlobal('open', vi.fn().mockReturnValue({ closed: false }));
  });

  it('launches Strava OAuth (not the Sciotte modal) while seats remain', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [stravaCard('oauth')] });
    const user = userEvent.setup();
    renderCards();

    const button = await screen.findByLabelText('Connect to Strava');
    await user.click(button);

    // Asserting the URL, not just that something was opened: a window opened on
    // `about:blank` is exactly the regression this replaced.
    await waitFor(() =>
      expect(window.open).toHaveBeenCalledWith('/api/oauth/authorize/strava', '_blank'),
    );
    expect(screen.queryByTestId('sciotte-modal')).not.toBeInTheDocument();
  });

  it('opens the Sciotte modal directly (no OAuth) once the pool is exhausted', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [stravaCard('mirror')] });
    const user = userEvent.setup();
    renderCards();

    const button = await screen.findByLabelText('Connect to Strava');
    await user.click(button);

    expect(await screen.findByTestId('sciotte-modal')).toHaveTextContent('strava');
    expect(window.open).not.toHaveBeenCalled();
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

  describe('the OAuth-callback poll is transient', () => {
    beforeEach(() => {
      vi.useFakeTimers({ shouldAdvanceTime: true });
    });

    afterEach(() => {
      vi.useRealTimers();
    });

    it('keeps asking while no grant has landed — the callback lands in another tab', async () => {
      getProvidersStatus.mockResolvedValue({ providers: [stravaCard('oauth')] });
      renderCards();
      await waitFor(() => expect(getProvidersStatus).toHaveBeenCalledTimes(1));

      await act(async () => {
        await vi.advanceTimersByTimeAsync(PROVIDER_LINK_POLL_INTERVAL_MS * 3);
      });
      await waitFor(() => expect(getProvidersStatus.mock.calls.length).toBeGreaterThan(1));
    });

    it('stops the moment a connection lands, instead of ticking for the life of the screen', async () => {
      getProvidersStatus.mockResolvedValue({ providers: [stravaCard('oauth')] });
      renderCards();
      await waitFor(() => expect(getProvidersStatus).toHaveBeenCalledTimes(1));

      // The grant completes in the other tab; the next poll sees it.
      getProvidersStatus.mockResolvedValue({
        providers: [{ ...stravaCard('oauth'), connected: true }],
      });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(PROVIDER_LINK_POLL_INTERVAL_MS);
      });
      await waitFor(() => expect(screen.getByText('Connected')).toBeInTheDocument());

      const callsWhenLanded = getProvidersStatus.mock.calls.length;
      await act(async () => {
        await vi.advanceTimersByTimeAsync(PROVIDER_LINK_POLL_INTERVAL_MS * 20);
      });
      expect(getProvidersStatus).toHaveBeenCalledTimes(callsWhenLanded);
    });
  });
});
