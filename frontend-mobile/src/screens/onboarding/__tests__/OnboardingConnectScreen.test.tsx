// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the mobile OnboardingConnectScreen Strava OAuth failure fallback
// ABOUTME: Verifies a failed Strava OAuth attempt falls back to the Sciotte credential login instead of stranding the first-run user

import React from 'react';
import { render, screen, fireEvent, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { OnboardingConnectScreen } from '../OnboardingConnectScreen';
import { oauthApi } from '../../../services/api';
import * as WebBrowser from 'expo-web-browser';
import * as Linking from 'expo-linking';

jest.mock('expo-web-browser', () => ({ openAuthSessionAsync: jest.fn() }));
jest.mock('expo-linking', () => ({ parse: jest.fn() }));
jest.mock('../../../utils/oauth', () => ({ getOAuthCallbackUrl: () => 'pierre://oauth-callback' }));
jest.mock('../../../contexts/AuthContext', () => ({
  useAuth: () => ({ isAuthenticated: true, user: { id: 'u1', display_name: 'Jean' }, logout: jest.fn() }),
}));
jest.mock('../../../services/api', () => ({
  oauthApi: { getProvidersStatus: jest.fn(), initMobileOAuth: jest.fn() },
}));
jest.mock('../../../components/SciotteLoginModal', () => {
  const React = require('react');
  const { Text } = require('react-native');
  return {
    SciotteLoginModal: ({ visible, target }: { visible: boolean; target: string }) =>
      visible ? React.createElement(Text, null, `sciotte-modal:${target}`) : null,
  };
});
jest.mock('../../../components/IntervalsIcuLinkModal', () => ({ IntervalsIcuLinkModal: () => null }));
jest.mock('../../../components/OAuthAppSetupModal', () => ({ OAuthAppSetupModal: () => null }));

const getProvidersStatus = oauthApi.getProvidersStatus as jest.Mock;
const initMobileOAuth = oauthApi.initMobileOAuth as jest.Mock;
const openAuthSessionAsync = WebBrowser.openAuthSessionAsync as jest.Mock;
const linkingParse = Linking.parse as jest.Mock;

const STRAVA_CARD = {
  provider: 'sciotte',
  display_name: 'Strava',
  requires_oauth: false,
  connected: false,
  needs_reauth: false,
  capabilities: ['activities'],
  recommended_backend: 'oauth' as const,
  seats_left: 3,
};

function renderScreen() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <OnboardingConnectScreen />
    </QueryClientProvider>,
  );
}

describe('OnboardingConnectScreen — Strava OAuth failure fallback', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    getProvidersStatus.mockResolvedValue({ providers: [STRAVA_CARD] });
    initMobileOAuth.mockResolvedValue({ authorization_url: 'https://www.strava.com/oauth/authorize' });
  });

  it('falls back to the Sciotte modal when the Strava OAuth callback returns an error', async () => {
    openAuthSessionAsync.mockResolvedValue({ type: 'success', url: 'pierre://oauth-callback?success=false&error=cap_exceeded' });
    linkingParse.mockReturnValue({ queryParams: { success: 'false', error: 'cap_exceeded' } });

    renderScreen();
    const connect = await screen.findByLabelText('Connect Strava');
    fireEvent.press(connect);

    expect(await screen.findByText('sciotte-modal:strava')).toBeTruthy();
  });

  it('falls back to the Sciotte modal when the Strava OAuth flow cannot start', async () => {
    initMobileOAuth.mockRejectedValue(new Error('network down'));

    renderScreen();
    const connect = await screen.findByLabelText('Connect Strava');
    fireEvent.press(connect);

    expect(await screen.findByText('sciotte-modal:strava')).toBeTruthy();
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
