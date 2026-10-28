// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the mobile ConnectionsScreen OAuth-first-with-Sciotte-fallback behavior
// ABOUTME: Verifies the Strava card defaults to OAuth while seats remain, uses Sciotte when the pool is full, and falls back to Sciotte on OAuth failure

import React from 'react';
import { Alert } from 'react-native';
import { render, screen, fireEvent, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
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
  getOAuthCallbackUrl: () => 'dravr://oauth-callback',
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
    needs_reauth: false,
    capabilities: ['activities'],
    recommended_backend,
    seats_left: recommended_backend === 'oauth' ? 3 : 0,
  };
}

/** The user-facing Garmin row, which follows the server's recommendation like Strava's (carnet#1155). */
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

/** The screen's consent action is a React Query mutation, so it renders under a client. */
function queryWrapper() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return ({ children }: { children: React.ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
}

describe('ConnectionsScreen — TrainingPeaks', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('opens the TrainingPeaks login with its notice, never an OAuth session', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [{
    provider: 'sciotte_trainingpeaks',
    display_name: 'TrainingPeaks',
    requires_oauth: false,
    connected: false,
    needs_reauth: false,
    capabilities: ['activities'],
    consent_required: true,
  }] });

    render(<ConnectionsScreen />, { wrapper: queryWrapper() });
    fireEvent.press(await screen.findByText('Connect'));

    expect(await screen.findByText('sciotte-modal:trainingpeaks:consent')).toBeTruthy();
    expect(initMobileOAuth).not.toHaveBeenCalled();
  });
});

describe('ConnectionsScreen — OAuth-first with Sciotte fallback', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    initMobileOAuth.mockResolvedValue({ authorization_url: 'https://www.strava.com/oauth/authorize' });
  });

  it('launches Strava OAuth (not Sciotte) while seats remain', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [stravaCard('oauth')] });
    // Keep the browser session pending so we only assert the launch.
    openAuthSessionAsync.mockReturnValue(new Promise(() => {}));

    render(<ConnectionsScreen />, { wrapper: queryWrapper() });
    const connect = await screen.findByText('Connect');
    fireEvent.press(connect);

    await waitFor(() => expect(initMobileOAuth).toHaveBeenCalledWith('strava', 'dravr://oauth-callback', { tosConsent: false }));
    expect(screen.queryByText('sciotte-modal:strava')).toBeNull();
  });

  it('opens the Sciotte modal directly (no OAuth) once the pool is exhausted', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [stravaCard('mirror')] });

    render(<ConnectionsScreen />, { wrapper: queryWrapper() });
    const connect = await screen.findByText('Connect');
    fireEvent.press(connect);

    expect(await screen.findByText('sciotte-modal:strava')).toBeTruthy();
    expect(initMobileOAuth).not.toHaveBeenCalled();
  });

  it('launches Garmin OAuth (not the credential login) while Garmin seats remain', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [garminCard('oauth')] });
    openAuthSessionAsync.mockReturnValue(new Promise(() => {}));

    render(<ConnectionsScreen />, { wrapper: queryWrapper() });
    fireEvent.press(await screen.findByText('Connect'));

    await waitFor(() => expect(initMobileOAuth).toHaveBeenCalledWith('garmin', 'dravr://oauth-callback', { tosConsent: false }));
    expect(screen.queryByText(/sciotte-modal:garmin/)).toBeNull();
  });

  it('opens the Garmin credential login once the Garmin pool is exhausted', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [garminCard('mirror')] });

    render(<ConnectionsScreen />, { wrapper: queryWrapper() });
    fireEvent.press(await screen.findByText('Connect'));

    expect(await screen.findByText(/sciotte-modal:garmin/)).toBeTruthy();
    expect(initMobileOAuth).not.toHaveBeenCalled();
  });

  it('falls back to the Sciotte modal when the Strava OAuth callback returns an error', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [stravaCard('oauth')] });
    openAuthSessionAsync.mockResolvedValue({ type: 'success', url: 'dravr://oauth-callback?success=false&error=cap_exceeded' });
    linkingParse.mockReturnValue({ queryParams: { success: 'false', error: 'cap_exceeded' } });

    render(<ConnectionsScreen />, { wrapper: queryWrapper() });
    const connect = await screen.findByText('Connect');
    fireEvent.press(connect);

    expect(await screen.findByText('sciotte-modal:strava')).toBeTruthy();
  });
  it('offers to start fresh, never the Sciotte fallback, when the account was a deleted account\'s', async () => {
    const alert = jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
    getProvidersStatus.mockResolvedValue({ providers: [stravaCard('oauth')] });
    openAuthSessionAsync.mockResolvedValue({
      type: 'success',
      url: 'dravr://oauth-callback?success=false&error=import_tombstoned&tombstone_id=tomb-1&deleted_at=2026-10-20T08%3A00%3A00%2B00%3A00',
    });
    linkingParse.mockReturnValue({
      queryParams: {
        success: 'false',
        error: 'import_tombstoned',
        tombstone_id: 'tomb-1',
        deleted_at: '2026-10-20T08:00:00+00:00',
      },
    });

    render(<ConnectionsScreen />, { wrapper: queryWrapper() });
    fireEvent.press(await screen.findByText('Connect'));

    await waitFor(() => expect(alert).toHaveBeenCalledTimes(1));
    expect(alert.mock.calls[0][0]).toBe('Strava');
    expect(alert.mock.calls[0][1]).toBe(
      'This Strava account belonged to a Dravr account deleted on Oct 20, 2026. Its data stays deleted until you choose to start fresh with it.'
    );
    expect(screen.queryByText('sciotte-modal:strava')).toBeNull();
    alert.mockRestore();
  });
});
