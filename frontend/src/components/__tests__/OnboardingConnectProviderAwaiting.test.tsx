// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the web first-run connect gate's awaiting-consent overlay and its 90 s give-up notice
// ABOUTME: Both are whole sentences in the app language naming the provider's brand, never English assembled in code

import { describe, it, expect, afterEach, vi } from 'vitest';
import { act, fireEvent, render, screen } from '@testing-library/react';
import { i18n } from '@pierre/i18n';
import OnboardingConnectProvider from '../OnboardingConnectProvider';

vi.mock('../../services/api', () => ({
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

describe('OnboardingConnectProvider — awaiting the provider consent', () => {
  afterEach(async () => {
    vi.useRealTimers();
    await i18n.changeLanguage('en');
  });

  it('says what it waits for in French, then gives up after 90 s in French', async () => {
    await i18n.changeLanguage('fr');
    vi.useFakeTimers();
    render(<OnboardingConnectProvider userDisplayName="Jean" />);

    fireEvent.click(screen.getByText('launch'));

    expect(screen.getByText('En attente de l’autorisation de Strava…')).toBeInTheDocument();
    expect(
      screen.getByText(
        'Termine l’autorisation dans la fenêtre pop-up. On te ramène au tableau de bord automatiquement dès que Strava aura confirmé.',
      ),
    ).toBeInTheDocument();

    act(() => {
      vi.advanceTimersByTime(90_000);
    });

    expect(screen.getByRole('alert')).toHaveTextContent(
      'Aucune réponse de Strava en 90 secondes. Si la fenêtre pop-up est encore ouverte, termine l’autorisation là-bas ; sinon, réessaie.',
    );
  });
});
