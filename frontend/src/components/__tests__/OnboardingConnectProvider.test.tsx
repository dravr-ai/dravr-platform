// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the web first-run connect gate's post-connect spinner
// ABOUTME: The sentence names the provider that connected; a coach who does not train sees only the coaching platforms

import { describe, it, expect, afterEach, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
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

function renderGate(props: { coachOnly?: boolean } = {}) {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <ThemeProvider>
        <OnboardingConnectProvider
          userDisplayName="Jean"
          onContinueWithoutProvider={() => {}}
          {...props}
        />
      </ThemeProvider>
    </QueryClientProvider>,
  );
}

describe('OnboardingConnectProvider — the post-connect spinner', () => {
  afterEach(async () => {
    await i18n.changeLanguage('en');
  });

  it('names Strava as connected while the dashboard prepares, in French', async () => {
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
      ],
    });
    await i18n.changeLanguage('fr');

    renderGate();

    expect(
      await screen.findByText('Strava connecté — préparation de ton tableau de bord…'),
    ).toBeInTheDocument();
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

describe('OnboardingConnectProvider — a coach who does not train', () => {
  const served = {
    providers: [
      card('sciotte', 'Strava'),
      card('sciotte_garmin', 'Garmin'),
      card('sciotte_trainingpeaks', 'TrainingPeaks'),
      card('whoop', 'WHOOP'),
      card('intervals_icu', 'Intervals.icu'),
    ],
  };

  it('is offered only TrainingPeaks and Intervals.icu, with the coach copy and no athlete preview', async () => {
    getProvidersStatus.mockResolvedValue(served);

    renderGate({ coachOnly: true });

    expect(await screen.findByText('TrainingPeaks')).toBeInTheDocument();
    expect(screen.getByText('Intervals.icu')).toBeInTheDocument();
    for (const hidden of ['Strava', 'Garmin', 'WHOOP']) {
      expect(screen.queryByText(hidden)).not.toBeInTheDocument();
    }
    expect(screen.getByText(i18n.t('onboarding.connectCoachPlatformIntro'))).toBeInTheDocument();
    expect(screen.getByText(i18n.t('onboarding.connectCoachPlatformLaterHint'))).toBeInTheDocument();
    expect(screen.queryByText(i18n.t('onboarding.connectProviderIntro'))).not.toBeInTheDocument();
    expect(screen.queryByText(i18n.t('shell.previewSeeExample'))).not.toBeInTheDocument();
  });

  it('an athlete still sees every served provider and the athlete copy', async () => {
    getProvidersStatus.mockResolvedValue(served);

    renderGate();

    expect(await screen.findByText('Strava')).toBeInTheDocument();
    expect(screen.getByText('WHOOP')).toBeInTheDocument();
    expect(screen.getByText(i18n.t('onboarding.connectProviderIntro'))).toBeInTheDocument();
    expect(screen.getByText(i18n.t('shell.previewSeeExample'))).toBeInTheDocument();
  });
});
