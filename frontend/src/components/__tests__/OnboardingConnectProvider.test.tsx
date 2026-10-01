// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the web first-run connect gate's post-connect spinner
// ABOUTME: The sentence names the provider that connected, in the app language, rather than a generic "Provider"

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

function renderGate() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <ThemeProvider>
        <OnboardingConnectProvider userDisplayName="Jean" />
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
