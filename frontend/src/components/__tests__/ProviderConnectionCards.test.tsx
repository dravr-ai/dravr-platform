// ABOUTME: Unit tests for ProviderConnectionCards OAuth-first-with-Sciotte-fallback behavior
// ABOUTME: Covers the seat-gated OAuth default, the mirror path, the failed-OAuth fallback and the own-app setup
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen, waitFor, act } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { PROVIDER_LINK_POLL_INTERVAL_MS } from '@pierre/shared-constants';
import ProviderConnectionCards from '../ProviderConnectionCards';
import { ThemeProvider } from '../../hooks/useTheme';

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
  default: ({
    isOpen,
    target,
    consentRequired,
    onOAuthLaunched,
  }: {
    isOpen: boolean;
    target: string;
    consentRequired?: boolean;
    onOAuthLaunched?: (target: string) => void;
  }) =>
    isOpen ? (
      <div data-testid="sciotte-modal" data-consent={String(Boolean(consentRequired))}>
        {target}
        <button type="button" onClick={() => onOAuthLaunched?.(target)}>
          oauth-launched
        </button>
      </div>
    ) : null,
}));
vi.mock('../IntervalsIcuLinkModal', () => ({
  default: () => null,
}));
// The own-app setup, reduced to the callback it shows and the Save that
// resumes the OAuth start.
vi.mock('../OAuthAppSetupModal', () => ({
  default: ({ callbackUrl, onSaved }: { callbackUrl?: string; onSaved: () => void }) => (
    <div data-testid="own-app-setup">
      {callbackUrl}
      <button type="button" onClick={onSaved}>
        own-app-save
      </button>
    </div>
  ),
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

function renderCards(props: { onOAuthLaunched?: (providerName: string) => void } = {}) {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <ThemeProvider>
        <ProviderConnectionCards {...props} />
      </ThemeProvider>
    </QueryClientProvider>,
  );
}

describe('ProviderConnectionCards — the glyph ink follows the scheme', () => {
  const card = (provider: string, display_name: string) => ({
    provider,
    display_name,
    description: `${display_name}, as the server describes it`,
    requires_oauth: false,
    connected: false,
    needs_reauth: false,
    capabilities: ['activities'],
    consent_required: false,
  });
  const providers = [
    card('sciotte_trainingpeaks', 'TrainingPeaks'),
    card('whoop', 'WHOOP'),
    card('sciotte', 'Strava'),
    card('synthetic', 'Synthetic'),
  ];

  beforeEach(() => {
    vi.clearAllMocks();
    getProvidersStatus.mockResolvedValue({ providers });
  });

  afterEach(() => {
    localStorage.removeItem('dravr.theme');
    document.documentElement.classList.remove('dark');
  });

  it('draws TrainingPeaks in its blue and WHOOP in body ink on the light canvas', async () => {
    localStorage.setItem('dravr.theme', 'light');
    renderCards();

    const trainingPeaks = await screen.findByTestId('provider-glyph-sciotte_trainingpeaks');
    expect(trainingPeaks).toHaveStyle({ color: 'rgb(0, 86, 149)' });

    // WHOOP's green is 1.83:1 on light paper: the glyph keeps the body ink
    // class and carries no colour of its own.
    const whoop = screen.getByTestId('provider-glyph-whoop');
    expect(whoop).toHaveClass('text-on-surface');
    expect(whoop.style.color).toBe('');

    expect(screen.getByTestId('provider-glyph-sciotte')).toHaveStyle({ color: 'rgb(252, 76, 2)' });
  });

  it('draws TrainingPeaks in body ink and WHOOP in its green on the dark canvas', async () => {
    localStorage.setItem('dravr.theme', 'dark');
    renderCards();

    // TrainingPeaks' blue is 2.46:1 on the dark canvas.
    const trainingPeaks = await screen.findByTestId('provider-glyph-sciotte_trainingpeaks');
    expect(trainingPeaks).toHaveClass('text-on-surface');
    expect(trainingPeaks.style.color).toBe('');

    expect(screen.getByTestId('provider-glyph-whoop')).toHaveStyle({ color: 'rgb(0, 212, 106)' });
    expect(screen.getByTestId('provider-glyph-sciotte')).toHaveStyle({ color: 'rgb(252, 76, 2)' });
  });

  it('shows each card with the description the server serves', async () => {
    renderCards();

    expect(await screen.findByText('WHOOP, as the server describes it')).toBeInTheDocument();
    expect(screen.getByText('Strava, as the server describes it')).toBeInTheDocument();
    expect(screen.getByLabelText('Synthetic - Synthetic, as the server describes it')).toBeInTheDocument();
  });

  it('draws a provider with no brand colour in body ink in either scheme', async () => {
    for (const scheme of ['light', 'dark']) {
      localStorage.setItem('dravr.theme', scheme);
      const { unmount } = renderCards();
      const glyph = await screen.findByTestId('provider-glyph-synthetic');
      expect(glyph).toHaveClass('text-on-surface');
      expect(glyph.style.color).toBe('');
      unmount();
    }
  });
});

describe('ProviderConnectionCards — TrainingPeaks', () => {
  const trainingPeaksCard = (consent_required: boolean) => ({
    provider: 'sciotte_trainingpeaks',
    display_name: 'TrainingPeaks',
    requires_oauth: false,
    connected: false,
    needs_reauth: false,
    capabilities: ['activities'],
    consent_required,
  });

  beforeEach(() => {
    vi.clearAllMocks();
    vi.stubGlobal('open', vi.fn());
  });

  it('opens the TrainingPeaks login, carrying whether its notice is still owed', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [trainingPeaksCard(true)] });
    const user = userEvent.setup();
    renderCards();

    await user.click(await screen.findByLabelText('Connect to TrainingPeaks'));

    const modal = await screen.findByTestId('sciotte-modal');
    expect(modal).toHaveTextContent('trainingpeaks');
    expect(modal).toHaveAttribute('data-consent', 'true');
    expect(window.open).not.toHaveBeenCalled();
  });

  it('does not ask again once the account has accepted it', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [trainingPeaksCard(false)] });
    const user = userEvent.setup();
    renderCards();

    await user.click(await screen.findByLabelText('Connect to TrainingPeaks'));

    expect(await screen.findByTestId('sciotte-modal')).toHaveAttribute('data-consent', 'false');
  });
});

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

  it('reports a launched OAuth window by the brand, never the target id', async () => {
    // The awaiting overlay printed the id it was handed, capitalised in code.
    getProvidersStatus.mockResolvedValue({ providers: [stravaCard('mirror')] });
    const onOAuthLaunched = vi.fn();
    const user = userEvent.setup();
    renderCards({ onOAuthLaunched });

    await user.click(await screen.findByLabelText('Connect to Strava'));
    await user.click(await screen.findByText('oauth-launched'));

    expect(onOAuthLaunched).toHaveBeenCalledWith('Strava');
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

describe('ProviderConnectionCards — an app of the athlete\'s own', () => {
  const whoopCard = (own_app_required: boolean) => ({
    provider: 'whoop',
    display_name: 'WHOOP',
    requires_oauth: true,
    connected: false,
    needs_reauth: false,
    capabilities: ['sleep', 'recovery'],
    consent_required: false,
    own_app_required,
    oauth_callback_url: 'https://app.dravr.ai/api/oauth/callback/whoop',
  });

  beforeEach(() => {
    vi.clearAllMocks();
    vi.stubGlobal('open', vi.fn().mockReturnValue({ closed: false }));
  });

  it('asks for the athlete\'s own app before the OAuth start while no app of the server can authorize it', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [whoopCard(true)] });
    const user = userEvent.setup();
    renderCards();

    await user.click(await screen.findByLabelText('Connect to WHOOP'));

    expect(await screen.findByTestId('own-app-setup')).toHaveTextContent(
      'https://app.dravr.ai/api/oauth/callback/whoop',
    );
    expect(window.open).not.toHaveBeenCalled();

    await user.click(screen.getByText('own-app-save'));
    expect(window.open).toHaveBeenCalledWith('/api/oauth/authorize/whoop', '_blank');
  });

  it('starts the OAuth flow directly while an app of the server can authorize it', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [whoopCard(false)] });
    const user = userEvent.setup();
    renderCards();

    await user.click(await screen.findByLabelText('Connect to WHOOP'));

    expect(window.open).toHaveBeenCalledWith('/api/oauth/authorize/whoop', '_blank');
    expect(screen.queryByTestId('own-app-setup')).toBeNull();
  });
});
