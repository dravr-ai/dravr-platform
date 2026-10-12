// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the web first-run connect gate's awaiting-consent overlay, its 90 s give-up notice and its close
// ABOUTME: Sentences are whole, in the app language; a connection landing puts the user back on the list

import { describe, it, expect, afterEach, vi } from 'vitest';
import { act, fireEvent, render, screen } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { i18n } from '@pierre/i18n';
import OnboardingConnectProvider from '../OnboardingConnectProvider';
import { QUERY_KEYS } from '../../constants/queryKeys';

const getProvidersStatus = vi.fn();

vi.mock('../../services/api', () => ({
  providersApi: { getProvidersStatus: (...args: unknown[]) => getProvidersStatus(...args) },
  oauthApi: { authorizeUrl: (provider: string) => `/api/oauth/authorize/${provider}` },
}));
vi.mock('../../hooks/useAuth', () => ({ useAuth: () => ({ logout: vi.fn() }) }));
vi.mock('../ConnectPreview', () => ({ default: () => null }));
vi.mock('../OAuthAppSetupModal', () => ({ default: () => null }));
// The cards stand in for the Strava BYO popup: one press reports the launch
// the way `SciotteLoginModal` does.
vi.mock('../ProviderConnectionCards', () => ({
  default: ({ onOAuthLaunched }: { onOAuthLaunched?: (provider: string) => void }) => (
    <button type="button" onClick={() => onOAuthLaunched?.('Strava')}>
      launch
    </button>
  ),
}));

function status(connected: boolean) {
  return {
    providers: [
      {
        provider: 'sciotte',
        display_name: 'Strava',
        requires_oauth: false,
        connected,
        needs_reauth: false,
        capabilities: ['activities'],
      },
    ],
  };
}

function renderGate() {
  // Seeded and never stale, so no mount fetch lands on top of what a test sets.
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: Infinity } },
  });
  queryClient.setQueryData(QUERY_KEYS.providers.status(), status(false));
  const view = render(
    <QueryClientProvider client={queryClient}>
      <OnboardingConnectProvider userDisplayName="Jean" />
    </QueryClientProvider>,
  );
  return { ...view, queryClient };
}

describe('OnboardingConnectProvider — awaiting the provider consent', () => {
  afterEach(async () => {
    vi.useRealTimers();
    await i18n.changeLanguage('en');
  });

  it('says what it waits for in French, then gives up after 90 s in French', async () => {
    await i18n.changeLanguage('fr');
    vi.useFakeTimers();
    getProvidersStatus.mockResolvedValue(status(false));
    renderGate();

    fireEvent.click(screen.getByText('launch'));

    expect(screen.getByText('En attente de l’autorisation de Strava…')).toBeInTheDocument();
    expect(
      screen.getByText(i18n.t('onboarding.finishAuthInPopup', { provider: 'Strava' })),
    ).toBeInTheDocument();

    act(() => {
      vi.advanceTimersByTime(90_000);
    });

    expect(screen.getByRole('alert')).toHaveTextContent(
      'Aucune réponse de Strava en 90 secondes. Si la fenêtre pop-up est encore ouverte, termine l’autorisation là-bas ; sinon, réessaie.',
    );
  });

  it('closes when a connection lands, putting the user back on the list', async () => {
    getProvidersStatus.mockResolvedValue(status(false));
    const { queryClient } = renderGate();

    fireEvent.click(screen.getByText('launch'));
    expect(screen.getByText(i18n.t('onboarding.awaitingProviderConsent', { provider: 'Strava' }))).toBeInTheDocument();

    act(() => {
      queryClient.setQueryData(QUERY_KEYS.providers.status(), status(true));
    });

    // React Query notifies on its own scheduler tick, so wait for the re-render.
    expect(await screen.findByText('launch')).toBeInTheDocument();
    expect(
      screen.queryByText(i18n.t('onboarding.awaitingProviderConsent', { provider: 'Strava' })),
    ).not.toBeInTheDocument();
  });
});
