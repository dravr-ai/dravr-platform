// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for Tes données, the web connect gate: it stays on the list after a first connection, with Continue
// ABOUTME: Three services lead and the rest fold behind « Plus de services »; the skip is a visible secondary, sign out quiet

import { describe, it, expect, afterEach, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { i18n } from '@pierre/i18n';
import OnboardingConnectProvider from '../OnboardingConnectProvider';
import { ThemeProvider } from '../../hooks/useTheme';

const getProvidersStatus = vi.fn();

vi.mock('../../services/api', () => ({
  providersApi: { getProvidersStatus: (...args: unknown[]) => getProvidersStatus(...args) },
  oauthApi: { authorizeUrl: (provider: string) => `/api/oauth/authorize/${provider}` },
}));
vi.mock('../../services/analytics', () => ({ track: vi.fn() }));
vi.mock('../../hooks/useAuth', () => ({ useAuth: () => ({ logout: vi.fn() }) }));

function renderGate(props: { onContinue?: () => void; onContinueWithoutProvider?: () => void } = {}) {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <ThemeProvider>
        <OnboardingConnectProvider
          onContinueWithoutProvider={() => {}}
          {...props}
        />
      </ThemeProvider>
    </QueryClientProvider>,
  );
}

describe('OnboardingConnectProvider — after a first connection', () => {
  afterEach(async () => {
    await i18n.changeLanguage('en');
  });

  it('keeps the list up to connect another service, and Continue moves on, in French', async () => {
    getProvidersStatus.mockResolvedValue({
      providers: [
        {
          provider: 'sciotte',
          display_name: 'Strava',
          requires_oauth: false,
          connected: true,
          needs_reauth: false,
          capabilities: ['activities'],
        },
        card('sciotte_garmin', 'Garmin'),
      ],
    });
    await i18n.changeLanguage('fr');
    const onContinue = vi.fn();

    renderGate({ onContinue });

    expect(await screen.findByText('Garmin')).toBeInTheDocument();
    expect(
      screen.getByText('Tu peux connecter un autre service maintenant, ou continuer.'),
    ).toBeInTheDocument();
    // Connected, so the "continue without connecting" escape is gone.
    expect(screen.queryByTestId('onboarding-connect-skip')).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Continuer' }));
    expect(onContinue).toHaveBeenCalledTimes(1);
  });
});

/** One unconnected card per provider the server serves on this screen. */
function card(provider: string, display_name: string) {
  return {
    provider,
    display_name,
    requires_oauth: provider === 'whoop',
    connected: false,
    needs_reauth: false,
    capabilities: ['activities'],
  };
}

describe('OnboardingConnectProvider — the athlete list', () => {
  const served = {
    providers: [
      card('sciotte', 'Strava'),
      card('sciotte_garmin', 'Garmin'),
      card('sciotte_trainingpeaks', 'TrainingPeaks'),
      card('whoop', 'WHOOP'),
      card('intervals_icu', 'Intervals.icu'),
    ],
  };

  it('leads with Garmin, Strava and Intervals.icu and folds the rest behind « Plus de services »', async () => {
    getProvidersStatus.mockResolvedValue(served);

    renderGate();

    expect(await screen.findByText('Strava')).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Connect your training' })).toBeInTheDocument();
    expect(screen.getByText(i18n.t('onboarding.connect.intro'))).toBeInTheDocument();
    const lead = ['provider-card-sciotte_garmin', 'provider-card-sciotte', 'provider-card-intervals_icu'];
    const cards = screen.getAllByTestId(/^provider-card-/).map((el) => el.getAttribute('data-testid'));
    expect(cards).toEqual(lead);

    const more = screen.getByTestId('provider-more-services');
    expect(more).toHaveAttribute('aria-expanded', 'false');
    expect(more).toHaveTextContent('More services');
    expect(more).toHaveTextContent('TrainingPeaks, WHOOP, file');
    expect(screen.queryByText('WHOOP', { selector: 'span.text-sm' })).not.toBeInTheDocument();

    fireEvent.click(more);
    expect(more).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByTestId('provider-card-whoop')).toBeInTheDocument();
    expect(screen.getByTestId('provider-card-sciotte_trainingpeaks')).toBeInTheDocument();
  });

  it('offers « Continue without connecting » as a visible secondary, and sign out stays quiet', async () => {
    getProvidersStatus.mockResolvedValue(served);
    const onContinueWithoutProvider = vi.fn();

    renderGate({ onContinueWithoutProvider });
    await screen.findByText('Strava');

    const skip = screen.getByTestId('onboarding-connect-skip');
    expect(skip).toHaveTextContent('Continue without connecting');
    expect(skip).toHaveClass('btn-secondary');
    fireEvent.click(skip);
    expect(onContinueWithoutProvider).toHaveBeenCalledTimes(1);

    const signOut = screen.getByRole('button', { name: i18n.t('common.logout') });
    expect(signOut).not.toHaveClass('btn-primary');
    expect(signOut).not.toHaveClass('btn-tertiary');
  });

  it('never says "the model": the agent reads the data', async () => {
    getProvidersStatus.mockResolvedValue(served);
    const { container } = renderGate();
    await screen.findByText('Strava');
    expect(container.textContent).not.toMatch(/\bmodel\b/i);
  });
});
