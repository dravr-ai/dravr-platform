// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the mobile ConnectionsScreen OAuth-first-with-Sciotte-fallback behavior
// ABOUTME: Verifies the Strava card defaults to OAuth while seats remain, uses Sciotte when the pool is full, and falls back to Sciotte on OAuth failure

import React from 'react';
import { render, screen, fireEvent, waitFor } from '@testing-library/react-native';
import { ConnectionsScreen } from '../ConnectionsScreen';
import { oauthApi } from '../../../services/api';
import * as WebBrowser from 'expo-web-browser';
import * as Linking from 'expo-linking';

jest.mock('expo-web-browser', () => ({
  openAuthSessionAsync: jest.fn(),
}));
jest.mock('expo-linking', () => ({
  parse: jest.fn(),
}));
jest.mock('../../../utils/oauth', () => ({
  getOAuthCallbackUrl: () => 'pierre://oauth-callback',
}));
jest.mock('../../../contexts/AuthContext', () => ({
  useAuth: () => ({ isAuthenticated: true }),
}));
jest.mock('../../../services/api', () => ({
  oauthApi: {
    getProvidersStatus: jest.fn(),
    initMobileOAuth: jest.fn(),
    disconnectProvider: jest.fn(),
    disconnectIntervalsIcu: jest.fn(),
  },
}));
// Render the Sciotte modal as a visible marker carrying its target so a fallback
// (target="strava") is observable.
jest.mock('../../../components/SciotteLoginModal', () => {
  const React = require('react');
  const { Text } = require('react-native');
  return {
    SciotteLoginModal: ({ visible, target }: { visible: boolean; target: string }) =>
      visible ? React.createElement(Text, null, `sciotte-modal:${target}`) : null,
  };
});
jest.mock('../../../components/IntervalsIcuLinkModal', () => ({ IntervalsIcuLinkModal: () => null }));
jest.mock('../../../components/OAuthCredentialsSection', () => ({ OAuthCredentialsSection: () => null }));
jest.mock('../../../components/OAuthAppSetupModal', () => ({ OAuthAppSetupModal: () => null }));

const getProvidersStatus = oauthApi.getProvidersStatus as jest.Mock;
const initMobileOAuth = oauthApi.initMobileOAuth as jest.Mock;
const openAuthSessionAsync = WebBrowser.openAuthSessionAsync as jest.Mock;
const linkingParse = Linking.parse as jest.Mock;

function stravaCard(recommended_backend: 'oauth' | 'mirror') {
  return {
    provider: 'sciotte',
    display_name: 'Strava',
    requires_oauth: false,
    connected: false,
    capabilities: ['activities'],
    recommended_backend,
    seats_left: recommended_backend === 'oauth' ? 3 : 0,
  };
}

describe('ConnectionsScreen — OAuth-first with Sciotte fallback', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    initMobileOAuth.mockResolvedValue({ authorization_url: 'https://www.strava.com/oauth/authorize' });
  });

  it('launches Strava OAuth (not Sciotte) while seats remain', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [stravaCard('oauth')] });
    // Keep the browser session pending so we only assert the launch.
    openAuthSessionAsync.mockReturnValue(new Promise(() => {}));

    render(<ConnectionsScreen />);
    const connect = await screen.findByText('Connect');
    fireEvent.press(connect);

    await waitFor(() => expect(initMobileOAuth).toHaveBeenCalledWith('strava', 'pierre://oauth-callback'));
    expect(screen.queryByText('sciotte-modal:strava')).toBeNull();
  });

  it('opens the Sciotte modal directly (no OAuth) once the pool is exhausted', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [stravaCard('mirror')] });

    render(<ConnectionsScreen />);
    const connect = await screen.findByText('Connect');
    fireEvent.press(connect);

    expect(await screen.findByText('sciotte-modal:strava')).toBeTruthy();
    expect(initMobileOAuth).not.toHaveBeenCalled();
  });

  it('falls back to the Sciotte modal when the Strava OAuth callback returns an error', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [stravaCard('oauth')] });
    openAuthSessionAsync.mockResolvedValue({ type: 'success', url: 'pierre://oauth-callback?success=false&error=cap_exceeded' });
    linkingParse.mockReturnValue({ queryParams: { success: 'false', error: 'cap_exceeded' } });

    render(<ConnectionsScreen />);
    const connect = await screen.findByText('Connect');
    fireEvent.press(connect);

    expect(await screen.findByText('sciotte-modal:strava')).toBeTruthy();
  });
});
