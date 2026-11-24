// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mobile Connections screen asks Garmin's two consents before every Garmin connect and carries the ticked keys
// ABOUTME: Pins the OAuth start's consents, the sign-in's consents, Cancel starting nothing, and no sheet for Strava (carnet#1157, carnet#1390)

import React from 'react';
import { render, screen, fireEvent, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { ConnectionsScreen } from '../ConnectionsScreen';
import { oauthApi } from '../../../services/api';
import * as WebBrowser from 'expo-web-browser';

jest.mock('expo-web-browser', () => ({ openAuthSessionAsync: jest.fn() }));
jest.mock('expo-linking', () => ({ parse: jest.fn() }));
jest.mock('../../../utils/oauth', () => ({ getOAuthCallbackUrl: () => 'dravr://oauth-callback' }));
jest.mock('../../../contexts/AuthContext', () => ({ useAuth: () => ({ isAuthenticated: true }) }));
jest.mock('../../../services/api', () => ({
  oauthApi: {
    getProvidersStatus: jest.fn(),
    initMobileOAuth: jest.fn(),
    disconnectProvider: jest.fn(),
    disconnectIntervalsIcu: jest.fn(),
  },
}));
// The sign-in window, reduced to its target and the consents it was handed.
jest.mock('../../../components/SciotteLoginModal', () => {
  const React = require('react');
  const { Text } = require('react-native');
  return {
    SciotteLoginModal: ({ visible, target, consents }: { visible: boolean; target: string; consents?: readonly string[] }) =>
      visible
        ? React.createElement(Text, { testID: 'sciotte-modal' }, `${target}:${consents ? consents.join(',') || 'none-ticked' : 'not-asked'}`)
        : null,
  };
});
jest.mock('../../../components/IntervalsIcuLinkModal', () => ({ IntervalsIcuLinkModal: () => null }));
jest.mock('../../../components/OAuthCredentialsSection', () => ({ OAuthCredentialsSection: () => null }));
jest.mock('../../../components/OAuthAppSetupModal', () => ({ OAuthAppSetupModal: () => null }));

const getProvidersStatus = oauthApi.getProvidersStatus as jest.Mock;
const initMobileOAuth = oauthApi.initMobileOAuth as jest.Mock;
const openAuthSessionAsync = WebBrowser.openAuthSessionAsync as jest.Mock;

function card(provider: string, display_name: string, recommended_backend: 'oauth' | 'mirror') {
  return {
    provider,
    display_name,
    requires_oauth: false,
    connected: false,
    needs_reauth: false,
    capabilities: ['activities'],
    recommended_backend,
    seats_left: recommended_backend === 'oauth' ? 99 : 0,
    consent_required: false,
  };
}

function queryWrapper() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return ({ children }: { children: React.ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
}

describe('ConnectionsScreen — the Garmin consents at connect', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    initMobileOAuth.mockResolvedValue({ authorization_url: 'https://connect.garmin.com/oauth2Confirm' });
    openAuthSessionAsync.mockReturnValue(new Promise(() => {}));
  });

  it('asks both consents, unticked, before the Garmin OAuth, and carries the ticked ones to its start', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [card('sciotte_garmin', 'Garmin', 'oauth')] });
    render(<ConnectionsScreen />, { wrapper: queryWrapper() });
    fireEvent.press(await screen.findByText('Connect'));

    expect(await screen.findByTestId('connect-consent-sheet')).toBeTruthy();
    expect(initMobileOAuth).not.toHaveBeenCalled();
    expect(screen.getByTestId('connect-consent-box-garmin')).not.toBeChecked();
    expect(screen.getByTestId('connect-consent-box-garmin_workout_transfer')).not.toBeChecked();

    fireEvent.press(screen.getByTestId('connect-consent-box-garmin'));
    fireEvent.press(screen.getByTestId('connect-consent-box-garmin_workout_transfer'));
    fireEvent.press(screen.getByTestId('connect-consent-continue'));

    await waitFor(() =>
      expect(initMobileOAuth).toHaveBeenCalledWith('garmin', 'dravr://oauth-callback', {
        tosConsent: false,
        signInBlocked: false,
        consents: ['garmin', 'garmin_workout_transfer'],
      }),
    );
  });

  it('hands the ticked consents to the Garmin sign-in while no seat is free', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [card('sciotte_garmin', 'Garmin', 'mirror')] });
    render(<ConnectionsScreen />, { wrapper: queryWrapper() });
    fireEvent.press(await screen.findByText('Connect'));

    fireEvent.press(await screen.findByTestId('connect-consent-box-garmin_workout_transfer'));
    fireEvent.press(screen.getByTestId('connect-consent-continue'));

    expect(await screen.findByTestId('sciotte-modal')).toHaveTextContent('garmin:garmin_workout_transfer');
    expect(initMobileOAuth).not.toHaveBeenCalled();
  });

  it('starts nothing when the sheet is cancelled', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [card('sciotte_garmin', 'Garmin', 'oauth')] });
    render(<ConnectionsScreen />, { wrapper: queryWrapper() });
    fireEvent.press(await screen.findByText('Connect'));

    fireEvent.press(await screen.findByTestId('connect-consent-cancel'));

    expect(initMobileOAuth).not.toHaveBeenCalled();
    expect(screen.queryByTestId('sciotte-modal')).toBeNull();
  });

  it('asks a Strava connect for no consent', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [card('sciotte', 'Strava', 'oauth')] });
    initMobileOAuth.mockResolvedValue({ authorization_url: 'https://www.strava.com/oauth/authorize' });
    render(<ConnectionsScreen />, { wrapper: queryWrapper() });
    fireEvent.press(await screen.findByText('Connect'));

    await waitFor(() =>
      expect(initMobileOAuth).toHaveBeenCalledWith('strava', 'dravr://oauth-callback', { tosConsent: false, signInBlocked: false }),
    );
    expect(initMobileOAuth.mock.calls[0][2].consents).toBeUndefined();
    expect(screen.queryByTestId('connect-consent-sheet')).toBeNull();
  });
});
