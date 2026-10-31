// ABOUTME: Unit tests for ProviderConnectionCards OAuth-first-with-Sciotte-fallback behavior
// ABOUTME: Covers the seat-gated OAuth default (Strava, Garmin) started in this tab, the mirror path, the failed-OAuth fallback, the own-app setup and Intervals.icu's two paths
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
import { QUERY_KEYS } from '../../constants/queryKeys';

const authorizeUrl = vi.fn((provider: string) => `/api/oauth/authorize/${provider}`);
const getProvidersStatus = vi.fn();
// A connect goes to the server's launch route in this tab
// (`startOAuthInThisTab`); this stands in for the navigation.
const assign = vi.fn();

function stubNavigation() {
  // A live fake window, so the import window's "popup blocked" same-tab
  // fallback is not taken.
  vi.stubGlobal('open', vi.fn().mockReturnValue({ closed: false }));
  vi.stubGlobal('location', { ...window.location, assign });
}

afterEach(() => {
  vi.unstubAllGlobals();
});

vi.mock('../../services/api', () => ({
  providersApi: { getProvidersStatus: (...args: unknown[]) => getProvidersStatus(...args) },
  oauthApi: { authorizeUrl: (...args: unknown[]) => authorizeUrl(...args) },
}));

vi.mock('../../services/analytics', () => ({ track: vi.fn() }));

// Render the Sciotte modal as a testid carrying its target so a fallback that
// opens it (target="strava" or "garmin") is observable.
vi.mock('../SciotteLoginModal', () => ({
  default: ({
    isOpen,
    target,
    consentRequired,
  }: {
    isOpen: boolean;
    target: string;
    consentRequired?: boolean;
  }) =>
    isOpen ? (
      <div data-testid="sciotte-modal" data-consent={String(Boolean(consentRequired))}>
        {target}
      </div>
    ) : null,
}));
// The Intervals.icu dialog, reduced to whether it was handed the
// authorization and the control that starts it.
vi.mock('../IntervalsIcuLinkModal', () => ({
  default: ({ isOpen, onAuthorize }: { isOpen: boolean; onAuthorize?: () => void }) =>
    isOpen ? (
      <div data-testid="intervals-modal" data-offers-authorize={String(onAuthorize !== undefined)}>
        {onAuthorize && (
          <button type="button" onClick={onAuthorize}>
            intervals-authorize
          </button>
        )}
      </div>
    ) : null,
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

// The `sciotte_garmin` card is the user-facing "Garmin" card; it follows the
// server's recommendation like Strava's (carnet#1155).
function garminCard(recommended_backend: 'oauth' | 'mirror') {
  return {
    provider: 'sciotte_garmin',
    display_name: 'Garmin',
    requires_oauth: false,
    connected: false,
    needs_reauth: false,
    capabilities: ['activities', 'sleep'],
    recommended_backend,
    seats_left: recommended_backend === 'oauth' ? 999 : 0,
    consent_required: false,
  };
}

function renderCards(queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })) {
  return render(
    <QueryClientProvider client={queryClient}>
      <ThemeProvider>
        <ProviderConnectionCards />
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
    stubNavigation();
  });

  it('opens the TrainingPeaks login, carrying whether its notice is still owed', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [trainingPeaksCard(true)] });
    const user = userEvent.setup();
    renderCards();

    await user.click(await screen.findByLabelText('Connect to TrainingPeaks'));

    const modal = await screen.findByTestId('sciotte-modal');
    expect(modal).toHaveTextContent('trainingpeaks');
    expect(modal).toHaveAttribute('data-consent', 'true');
    expect(assign).not.toHaveBeenCalled();
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
    sessionStorage.clear();
    stubNavigation();
  });

  it('launches Strava OAuth (not the Sciotte modal) while seats remain', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [stravaCard('oauth')] });
    const user = userEvent.setup();
    renderCards();

    const button = await screen.findByLabelText('Connect to Strava');
    await user.click(button);

    // Asserting the URL, not just that something navigated: and in this tab —
    // a second tab is the copy of the app the athlete was left with.
    await waitFor(() => expect(assign).toHaveBeenCalledWith('/api/oauth/authorize/strava'));
    expect(window.open).not.toHaveBeenCalled();
    expect(screen.queryByTestId('sciotte-modal')).not.toBeInTheDocument();
  });

  it('opens the Sciotte modal directly (no OAuth) once the pool is exhausted', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [stravaCard('mirror')] });
    const user = userEvent.setup();
    renderCards();

    const button = await screen.findByLabelText('Connect to Strava');
    await user.click(button);

    expect(await screen.findByTestId('sciotte-modal')).toHaveTextContent('strava');
    expect(assign).not.toHaveBeenCalled();
  });

  it('launches Garmin OAuth (not the credential login) while Garmin seats remain', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [garminCard('oauth')] });
    const user = userEvent.setup();
    renderCards();

    await user.click(await screen.findByLabelText('Connect to Garmin'));

    await waitFor(() => expect(assign).toHaveBeenCalledWith('/api/oauth/authorize/garmin'));
    expect(window.open).not.toHaveBeenCalled();
    expect(screen.queryByTestId('sciotte-modal')).not.toBeInTheDocument();
  });

  it('opens the Garmin credential login once the Garmin pool is exhausted', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [garminCard('mirror')] });
    const user = userEvent.setup();
    renderCards();

    await user.click(await screen.findByLabelText('Connect to Garmin'));

    expect(await screen.findByTestId('sciotte-modal')).toHaveTextContent('garmin');
    expect(assign).not.toHaveBeenCalled();
  });

  it('keeps the route the connect started from, for the return to this tab', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [garminCard('oauth')] });
    vi.stubGlobal('location', { ...window.location, hash: '#/settings/connections', assign });
    const user = userEvent.setup();
    renderCards();

    await user.click(await screen.findByLabelText('Connect to Garmin'));

    await waitFor(() => expect(assign).toHaveBeenCalledWith('/api/oauth/authorize/garmin'));
    expect(sessionStorage.getItem('pierre_oauth_return')).toBe('/settings/connections');
  });

  it('falls back to the Sciotte modal when a Strava OAuth attempt fails', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [stravaCard('oauth')] });
    renderCards();
    await screen.findByLabelText('Connect to Strava');

    // A result left by a connect finished elsewhere: a failed Strava connection.
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

  it('opens the Garmin credential login when the connect comes back refused', async () => {
    // The return to this tab leaves the result before the cards mount, so
    // they read it on mount rather than from a `storage` event.
    const failed = JSON.stringify({
      type: 'oauth_completed',
      provider: 'garmin',
      success: false,
      hasError: true,
      tombstoned: false,
      timestamp: Date.now(),
    });
    localStorage.setItem('pierre_oauth_result', failed);
    getProvidersStatus.mockResolvedValue({ providers: [garminCard('oauth')] });
    renderCards();

    expect(await screen.findByTestId('sciotte-modal')).toHaveTextContent('garmin');
    expect(localStorage.getItem('pierre_oauth_result')).toBeNull();
    expect(assign).not.toHaveBeenCalled();
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

  it('refetches the onboarding status when a connect comes back with a grant (carnet#1042)', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [stravaCard('oauth')] });
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const invalidate = vi.spyOn(queryClient, 'invalidateQueries');
    renderCards(queryClient);
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

    // Without it this tab keeps its stale "needs a provider" status and never
    // leaves the connect gate for the coach proposal.
    expect(invalidate).toHaveBeenCalledWith({ queryKey: QUERY_KEYS.user.onboardingStatus() });
  });

  describe('a Garmin grant with nothing pushed yet leads to the usr/pwd login (carnet#1302)', () => {
    // The Garmin card connected through its OAuth grant, whose store has
    // received no activity yet: what the server serves after a fresh grant.
    function garminAwaiting(awaiting: boolean) {
      return {
        ...garminCard('oauth'),
        connected: true,
        connected_backend: 'garmin',
        oauth_awaiting_activities: awaiting,
      };
    }
    function grant(provider: string) {
      return JSON.stringify({
        type: 'oauth_completed',
        provider,
        success: true,
        hasError: false,
        tombstoned: false,
        timestamp: Date.now(),
      });
    }

    it('opens the Garmin login once a Garmin grant comes back with no activities', async () => {
      localStorage.setItem('pierre_oauth_result', grant('garmin'));
      getProvidersStatus.mockResolvedValue({ providers: [garminAwaiting(true)] });
      renderCards();

      expect(await screen.findByTestId('sciotte-modal')).toHaveTextContent('garmin');
      // The grant is left for the success handlers, as every grant is.
      expect(localStorage.getItem('pierre_oauth_result')).not.toBeNull();
      expect(assign).not.toHaveBeenCalled();
    });

    it('stays closed when the Garmin grant already holds activities', async () => {
      localStorage.setItem('pierre_oauth_result', grant('garmin'));
      getProvidersStatus.mockResolvedValue({ providers: [garminAwaiting(false)] });
      renderCards();

      expect(await screen.findByLabelText('Garmin is connected')).toBeDisabled();
      expect(screen.queryByTestId('sciotte-modal')).not.toBeInTheDocument();
    });

    it('offers the Garmin login from the card of a grant still awaiting activities', async () => {
      getProvidersStatus.mockResolvedValue({ providers: [garminAwaiting(true)] });
      const user = userEvent.setup();
      renderCards();

      // The row names its state and what it offers, never a bare "Connect".
      expect(await screen.findByText('Connected — waiting for activities')).toBeInTheDocument();
      expect(screen.getByText('Import my history')).toBeInTheDocument();
      expect(screen.queryByLabelText('Connect to Garmin')).not.toBeInTheDocument();
      await user.click(screen.getByLabelText('Import my Garmin history'));

      expect(await screen.findByTestId('sciotte-modal')).toHaveTextContent('garmin');
      expect(assign).not.toHaveBeenCalled();
    });
  });

  describe('the OAuth-callback poll is transient', () => {
    beforeEach(() => {
      vi.useFakeTimers({ shouldAdvanceTime: true });
    });

    afterEach(() => {
      vi.useRealTimers();
    });

    it('keeps asking while no grant has landed — one can land where this screen does not see', async () => {
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

      // The grant completes elsewhere (a chat's hosted connect page); the next poll sees it.
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
    stubNavigation();
  });

  it('asks for the athlete\'s own app before the OAuth start while no app of the server can authorize it', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [whoopCard(true)] });
    const user = userEvent.setup();
    renderCards();

    await user.click(await screen.findByLabelText('Connect to WHOOP'));

    expect(await screen.findByTestId('own-app-setup')).toHaveTextContent(
      'https://app.dravr.ai/api/oauth/callback/whoop',
    );
    expect(assign).not.toHaveBeenCalled();

    await user.click(screen.getByText('own-app-save'));
    expect(assign).toHaveBeenCalledWith('/api/oauth/authorize/whoop');
    expect(window.open).not.toHaveBeenCalled();
  });

  it('starts the OAuth flow directly while an app of the server can authorize it', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [whoopCard(false)] });
    const user = userEvent.setup();
    renderCards();

    await user.click(await screen.findByLabelText('Connect to WHOOP'));

    expect(assign).toHaveBeenCalledWith('/api/oauth/authorize/whoop');
    expect(window.open).not.toHaveBeenCalled();
    expect(screen.queryByTestId('own-app-setup')).toBeNull();
  });
});

describe('ProviderConnectionCards — Intervals.icu', () => {
  const intervalsCard = (own_app_required: boolean) => ({
    provider: 'intervals_icu',
    display_name: 'Intervals.icu',
    requires_oauth: true,
    connected: false,
    needs_reauth: false,
    capabilities: ['activities', 'health'],
    consent_required: false,
    own_app_required,
    oauth_callback_url: 'https://app.dravr.ai/api/oauth/callback/intervals_icu',
  });

  beforeEach(() => {
    vi.clearAllMocks();
    stubNavigation();
  });

  it('leads with the authorization while the server offers it, and starts it from the dialog', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [intervalsCard(false)] });
    const user = userEvent.setup();
    renderCards();

    await user.click(await screen.findByLabelText('Connect to Intervals.icu'));

    expect(await screen.findByTestId('intervals-modal')).toHaveAttribute('data-offers-authorize', 'true');
    expect(assign).not.toHaveBeenCalled();

    await user.click(screen.getByText('intervals-authorize'));

    expect(assign).toHaveBeenCalledWith('/api/oauth/authorize/intervals_icu');
    expect(window.open).not.toHaveBeenCalled();
    expect(screen.queryByTestId('intervals-modal')).toBeNull();
  });

  it('opens the key form alone, never an own-app setup, while no client resolves', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [intervalsCard(true)] });
    const user = userEvent.setup();
    renderCards();

    await user.click(await screen.findByLabelText('Connect to Intervals.icu'));

    expect(await screen.findByTestId('intervals-modal')).toHaveAttribute('data-offers-authorize', 'false');
    expect(screen.queryByTestId('own-app-setup')).toBeNull();
    expect(assign).not.toHaveBeenCalled();
  });
});

// JF, 2026-10-27: "We will not drop scrape at all, this is major." Importing
// your own data arrives beside the connections, never in place of one: the
// Strava and Garmin credential logins stay offered on the same cards, and
// the import link is one more row under them.
describe('ProviderConnectionCards — importing your own data sits beside the credential login', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    stubNavigation();
  });

  it('keeps the Strava and Garmin credential logins when the seats are gone, with the import link below them', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [stravaCard('mirror'), garminCard('mirror')] });
    const user = userEvent.setup();
    renderCards();

    const strava = await screen.findByLabelText('Connect to Strava');
    const link = screen.getByTestId('import-own-data-link');
    expect(
      strava.compareDocumentPosition(link) & Node.DOCUMENT_POSITION_FOLLOWING,
      'the import link comes after the provider rows',
    ).toBeTruthy();

    await user.click(strava);
    expect(await screen.findByTestId('sciotte-modal')).toHaveTextContent('strava');

    await user.click(screen.getByLabelText('Connect to Garmin'));
    expect(await screen.findByTestId('sciotte-modal')).toHaveTextContent('garmin');
  });

  it('opens the import window from the link, not a connection', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [stravaCard('mirror')] });
    const user = userEvent.setup();
    renderCards();

    await user.click(await screen.findByTestId('import-own-data-link'));

    expect(window.open).toHaveBeenCalledWith('/import', '_blank');
    expect(assign).not.toHaveBeenCalled();
    expect(screen.queryByTestId('sciotte-modal')).not.toBeInTheDocument();
  });
});
