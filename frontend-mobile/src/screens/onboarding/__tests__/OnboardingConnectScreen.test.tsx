// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the mobile OnboardingConnectScreen: the Strava OAuth failure fallback, the own-app setup, Intervals.icu's two paths and the coach-only offer
// ABOUTME: A failed Strava OAuth falls back to the Sciotte credential login; a coach who does not train sees only the coaching platforms

import React from 'react';
import { render, screen, fireEvent, waitFor, act } from '@testing-library/react-native';
import { i18n } from '@pierre/i18n';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { OnboardingConnectScreen } from '../OnboardingConnectScreen';
import { oauthApi } from '../../../services/api';
import * as WebBrowser from 'expo-web-browser';
import * as Linking from 'expo-linking';

jest.mock('expo-web-browser', () => ({ openAuthSessionAsync: jest.fn() }));
jest.mock('expo-linking', () => ({ parse: jest.fn() }));
jest.mock('../../../utils/oauth', () => ({ getOAuthCallbackUrl: () => 'dravr://oauth-callback' }));
jest.mock('../../../contexts/AuthContext', () => ({
  useAuth: () => ({ isAuthenticated: true, user: { id: 'u1', display_name: 'Jean' }, logout: jest.fn() }),
}));
jest.mock('../../../hooks/useOnboardingProgress', () => ({
  useOnboardingProgress: () => [
    { id: 'connect_provider', labelKey: 'onboarding.stepConnect', status: 'current' },
  ],
}));
// Whether the account is a coach who does not train; flipped per test.
let mockCoachOnly = false;
jest.mock('../../../hooks/useOnboardingContext', () => ({
  useOnboardingContext: () => ({ context: { athleteStepsWaived: mockCoachOnly } }),
}));
jest.mock('../../../services/api', () => ({
  oauthApi: { getProvidersStatus: jest.fn(), initMobileOAuth: jest.fn() },
}));
jest.mock('../../../components/SciotteLoginModal', () => {
  const React = require('react');
  const { Text } = require('react-native');
  return {
    SciotteLoginModal: ({
      visible,
      target,
      consentRequired,
    }: {
      visible: boolean;
      target: string;
      consentRequired?: boolean;
    }) =>
      visible
        ? React.createElement(Text, null, `sciotte-modal:${target}${consentRequired ? ':consent' : ''}`)
        : null,
  };
});
// The Intervals.icu sheet, reduced to which path it opened on and the
// control that starts the authorization when it was handed one.
jest.mock('../../../components/IntervalsIcuLinkModal', () => {
  const React = require('react');
  const { Text } = require('react-native');
  return {
    IntervalsIcuLinkModal: ({ visible, onAuthorize }: { visible: boolean; onAuthorize?: () => void }) =>
      visible
        ? React.createElement(
            Text,
            { onPress: onAuthorize, testID: 'intervals-sheet' },
            onAuthorize ? 'intervals-sheet:authorize' : 'intervals-sheet:key',
          )
        : null,
  };
});
// The own-app setup sheet, reduced to the callback it shows and the Save that
// resumes the OAuth start.
jest.mock('../../../components/OAuthAppSetupModal', () => {
  const React = require('react');
  const { Text } = require('react-native');
  return {
    OAuthAppSetupModal: ({ callbackUrl, onSaved }: { callbackUrl?: string; onSaved: () => void }) =>
      React.createElement(
        React.Fragment,
        null,
        React.createElement(Text, null, `own-app-setup:${callbackUrl}`),
        React.createElement(Text, { onPress: onSaved }, 'own-app-save'),
      ),
  };
});

const getProvidersStatus = oauthApi.getProvidersStatus as jest.Mock;
const initMobileOAuth = oauthApi.initMobileOAuth as jest.Mock;
const openAuthSessionAsync = WebBrowser.openAuthSessionAsync as jest.Mock;
const linkingParse = Linking.parse as jest.Mock;

const STRAVA_CARD = {
  provider: 'sciotte',
  display_name: 'Strava',
  description: 'Activités de course, de vélo et de natation',
  requires_oauth: false,
  connected: false,
  needs_reauth: false,
  capabilities: ['activities'],
  recommended_backend: 'oauth' as const,
  seats_left: 3,
};

function renderScreen() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const view = render(
    <QueryClientProvider client={queryClient}>
      <OnboardingConnectScreen />
    </QueryClientProvider>,
  );
  return { ...view, queryClient };
}

describe('OnboardingConnectScreen — TrainingPeaks', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('opens the TrainingPeaks login with its notice, never an OAuth session', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [{
    provider: 'sciotte_trainingpeaks',
    display_name: 'TrainingPeaks',
    description: 'Completed workouts and their training load from TrainingPeaks',
    requires_oauth: false,
    connected: false,
    needs_reauth: false,
    capabilities: ['activities'],
    consent_required: true,
  }] });

    renderScreen();
    fireEvent.press(await screen.findByLabelText('Connect TrainingPeaks'));

    expect(await screen.findByText('sciotte-modal:trainingpeaks:consent')).toBeTruthy();
    expect(initMobileOAuth).not.toHaveBeenCalled();
  });
});

describe('OnboardingConnectScreen — a paused sign-in', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('says the sign-in is paused and offers no "Connect"', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [{
      provider: 'sciotte_garmin',
      display_name: 'Garmin',
      description: 'Garmin, as the server describes it',
      requires_oauth: false,
      connected: false,
      needs_reauth: false,
      capabilities: ['activities'],
      consent_required: false,
      connect_paused: true,
    }] });

    renderScreen();

    expect(await screen.findByTestId('provider-connect-paused-sciotte_garmin')).toHaveTextContent(
      "Garmin sign-in is paused while we move to Garmin's official connection.",
    );
    expect(screen.queryByLabelText('Connect Garmin')).toBeNull();
    expect(screen.queryByTestId('provider-action-sciotte_garmin')).toBeNull();
  });
});

describe('OnboardingConnectScreen — Strava OAuth failure fallback', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    getProvidersStatus.mockResolvedValue({ providers: [STRAVA_CARD] });
    initMobileOAuth.mockResolvedValue({ authorization_url: 'https://www.strava.com/oauth/authorize' });
  });

  it('falls back to the Sciotte modal when the Strava OAuth callback returns an error', async () => {
    openAuthSessionAsync.mockResolvedValue({ type: 'success', url: 'dravr://oauth-callback?success=false&error=cap_exceeded' });
    linkingParse.mockReturnValue({ queryParams: { success: 'false', error: 'cap_exceeded' } });

    renderScreen();
    const connect = await screen.findByLabelText('Connect Strava');
    fireEvent.press(connect);

    expect(await screen.findByText('sciotte-modal:strava')).toBeTruthy();
  });

  it('stays on the list after Strava connects, offering another service and Continue, in French', async () => {
    openAuthSessionAsync.mockResolvedValue({ type: 'success', url: 'dravr://oauth-callback?success=true' });
    linkingParse.mockReturnValue({ queryParams: { success: 'true' } });
    const garmin = { ...STRAVA_CARD, provider: 'sciotte_garmin', display_name: 'Garmin' };
    getProvidersStatus.mockResolvedValue({ providers: [STRAVA_CARD, garmin] });
    await i18n.changeLanguage('fr');
    try {
      const { queryClient } = renderScreen();
      fireEvent.press(await screen.findByLabelText('Connecter Strava'));
      // The refetch after the OAuth sheet closes sees Strava connected.
      getProvidersStatus.mockResolvedValue({
        providers: [{ ...STRAVA_CARD, connected: true }, garmin],
      });

      expect(
        await screen.findByText('Tu peux connecter un autre service maintenant, ou continuer.'),
      ).toBeTruthy();
      // The other services are still one tap away.
      expect(screen.getByLabelText('Connecter Garmin')).toBeTruthy();
      expect(screen.queryByText(i18n.t('app.continueWithoutConnecting'))).toBeNull();

      fireEvent.press(screen.getByText('Continuer'));
      expect(queryClient.getQueryData(['connect-provider-done', 'u1'])).toBe(true);
    } finally {
      await i18n.changeLanguage('en');
    }
  });

  it('says in French which brand it awaits while the auth sheet is open', async () => {
    // The sheet stays open until the test closes it: meanwhile the screen
    // shows its awaiting overlay.
    let closeSheet: (result: { type: 'cancel' }) => void = () => {};
    openAuthSessionAsync.mockReturnValue(
      new Promise((resolve) => {
        closeSheet = resolve;
      }),
    );
    await i18n.changeLanguage('fr');
    try {
      renderScreen();
      fireEvent.press(await screen.findByLabelText('Connecter Strava'));

      expect(await screen.findByText('En attente de l’autorisation de Strava…')).toBeTruthy();
      expect(
        screen.getByText(i18n.t('app.finishAuthInBrowser', { provider: 'Strava' })),
      ).toBeTruthy();
    } finally {
      await act(async () => {
        closeSheet({ type: 'cancel' });
      });
      await i18n.changeLanguage('en');
    }
  });

  it('falls back to the Sciotte modal when the Strava OAuth flow cannot start', async () => {
    initMobileOAuth.mockRejectedValue(new Error('network down'));

    renderScreen();
    const connect = await screen.findByLabelText('Connect Strava');
    fireEvent.press(connect);

    expect(await screen.findByText('sciotte-modal:strava')).toBeTruthy();
  });

  it('renders the provider row as a brand glyph + ink action, under the progress hairline', async () => {
    renderScreen();

    expect(screen.getByTestId('onboarding-progress-bar')).toBeTruthy();
    expect(await screen.findByTestId('provider-action-sciotte')).toHaveTextContent('Connect');
    // The line under the name is the one the server serves, as served.
    expect(screen.getByText('Activités de course, de vélo et de natation')).toBeTruthy();
    // The demoted Logout is its own quiet link, not the old full-width Button.
    expect(screen.getByTestId('onboarding-logout-link')).toBeTruthy();
  });

  it('does NOT fall back when the user cancels the auth sheet', async () => {
    openAuthSessionAsync.mockResolvedValue({ type: 'cancel' });

    renderScreen();
    const connect = await screen.findByLabelText('Connect Strava');
    fireEvent.press(connect);

    await waitFor(() => expect(openAuthSessionAsync).toHaveBeenCalled());
    expect(screen.queryByText('sciotte-modal:strava')).toBeNull();
  });
});

describe('OnboardingConnectScreen — an app of the athlete\'s own', () => {
  const whoop = (own_app_required: boolean) => ({
    provider: 'whoop',
    display_name: 'WHOOP',
    description: '',
    requires_oauth: true,
    connected: false,
    needs_reauth: false,
    capabilities: ['sleep', 'recovery'],
    consent_required: false,
    own_app_required,
    oauth_callback_url: 'https://app.dravr.ai/api/oauth/callback/whoop',
  });

  beforeEach(() => {
    jest.clearAllMocks();
    initMobileOAuth.mockResolvedValue({ authorization_url: 'https://api.prod.whoop.com/oauth/oauth2/auth' });
    openAuthSessionAsync.mockResolvedValue({ type: 'cancel' });
  });

  it('asks for the athlete\'s own app before the OAuth start while no app of the server can authorize it', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [whoop(true)] });
    renderScreen();
    fireEvent.press(await screen.findByLabelText('Connect WHOOP'));

    expect(
      await screen.findByText('own-app-setup:https://app.dravr.ai/api/oauth/callback/whoop'),
    ).toBeTruthy();
    expect(initMobileOAuth).not.toHaveBeenCalled();

    fireEvent.press(screen.getByText('own-app-save'));
    await waitFor(() =>
      expect(initMobileOAuth).toHaveBeenCalledWith('whoop', 'dravr://oauth-callback', { tosConsent: false }),
    );
  });

  it('starts the OAuth flow directly while an app of the server can authorize it', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [whoop(false)] });
    renderScreen();
    fireEvent.press(await screen.findByLabelText('Connect WHOOP'));

    await waitFor(() =>
      expect(initMobileOAuth).toHaveBeenCalledWith('whoop', 'dravr://oauth-callback', { tosConsent: false }),
    );
    expect(screen.queryByText(/own-app-setup/)).toBeNull();
  });
});

describe('OnboardingConnectScreen — Intervals.icu', () => {
  const intervalsCard = (own_app_required: boolean) => ({
    provider: 'intervals_icu',
    display_name: 'Intervals.icu',
    description: 'Endurance analytics, activities, and wellness',
    requires_oauth: true,
    connected: false,
    needs_reauth: false,
    capabilities: ['activities', 'health'],
    consent_required: false,
    own_app_required,
    oauth_callback_url: 'https://app.dravr.ai/api/oauth/callback/intervals_icu',
  });

  beforeEach(() => {
    jest.clearAllMocks();
    initMobileOAuth.mockResolvedValue({ authorization_url: 'https://intervals.icu/oauth/authorize' });
    openAuthSessionAsync.mockReturnValue(new Promise(() => {}));
  });

  it('leads with the authorization while the server offers it, and starts it from the sheet', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [intervalsCard(false)] });
    renderScreen();

    fireEvent.press(await screen.findByLabelText('Connect Intervals.icu'));
    expect(await screen.findByText('intervals-sheet:authorize')).toBeTruthy();
    expect(initMobileOAuth).not.toHaveBeenCalled();

    fireEvent.press(screen.getByTestId('intervals-sheet'));

    await waitFor(() =>
      expect(initMobileOAuth).toHaveBeenCalledWith('intervals_icu', 'dravr://oauth-callback', { tosConsent: false }),
    );
  });

  it('opens the key form alone, never an own-app setup, while no client resolves', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [intervalsCard(true)] });
    renderScreen();

    fireEvent.press(await screen.findByLabelText('Connect Intervals.icu'));

    expect(await screen.findByText('intervals-sheet:key')).toBeTruthy();
    expect(screen.queryByText(/own-app-setup/)).toBeNull();
    expect(initMobileOAuth).not.toHaveBeenCalled();
  });
});

describe('OnboardingConnectScreen — a coach who does not train', () => {
  const card = (provider: string, display_name: string) => ({
    ...STRAVA_CARD,
    provider,
    display_name,
    requires_oauth: provider === 'whoop',
  });
  const served = {
    providers: [
      card('sciotte', 'Strava'),
      card('sciotte_garmin', 'Garmin'),
      card('sciotte_trainingpeaks', 'TrainingPeaks'),
      card('whoop', 'WHOOP'),
      card('intervals_icu', 'Intervals.icu'),
    ],
  };

  beforeEach(() => {
    jest.clearAllMocks();
    getProvidersStatus.mockResolvedValue(served);
  });

  afterEach(() => {
    mockCoachOnly = false;
  });

  it('is offered only TrainingPeaks and Intervals.icu, with the coach copy', async () => {
    mockCoachOnly = true;

    renderScreen();

    expect(await screen.findByLabelText('Connect TrainingPeaks')).toBeTruthy();
    expect(screen.getByLabelText('Connect Intervals.icu')).toBeTruthy();
    for (const hidden of ['Strava', 'Garmin', 'WHOOP']) {
      expect(screen.queryByLabelText(`Connect ${hidden}`)).toBeNull();
    }
    expect(screen.getByText(i18n.t('onboarding.connectCoachPlatformIntro'))).toBeTruthy();
    expect(screen.getByText(i18n.t('onboarding.connectCoachPlatformLaterHint'))).toBeTruthy();
    expect(screen.queryByText(i18n.t('onboarding.connectProviderIntro'))).toBeNull();
  });

  it('an athlete still sees every provider an athlete connects', async () => {
    renderScreen();

    expect(await screen.findByLabelText('Connect Strava')).toBeTruthy();
    expect(screen.getByLabelText('Connect WHOOP')).toBeTruthy();
    expect(screen.getByText(i18n.t('onboarding.connectProviderIntro'))).toBeTruthy();
  });
});
