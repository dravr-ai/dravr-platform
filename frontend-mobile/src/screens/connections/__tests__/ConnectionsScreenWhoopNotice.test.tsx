// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mobile Connections screen states WHOOP's owner authorization before its OAuth flow and carries the acceptance
// ABOUTME: Pins the notice sheet, its held Continue, the tos_consent start, and a connected WHOOP that owes it asking again

import React from 'react';
import { render, screen, fireEvent, waitFor } from '@testing-library/react-native';
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
jest.mock('../../../components/SciotteLoginModal', () => ({ SciotteLoginModal: () => null }));
jest.mock('../../../components/IntervalsIcuLinkModal', () => ({ IntervalsIcuLinkModal: () => null }));
jest.mock('../../../components/OAuthCredentialsSection', () => ({ OAuthCredentialsSection: () => null }));
// The WHOOP app setup sheet, reduced to the Save that resumes the OAuth start.
jest.mock('../../../components/OAuthAppSetupModal', () => {
  const React = require('react');
  const { Text } = require('react-native');
  return {
    OAuthAppSetupModal: ({ visible, onSaved }: { visible: boolean; onSaved: () => void }) =>
      visible ? React.createElement(Text, { onPress: onSaved }, 'whoop-setup:save') : null,
  };
});

const getProvidersStatus = oauthApi.getProvidersStatus as jest.Mock;
const initMobileOAuth = oauthApi.initMobileOAuth as jest.Mock;
const openAuthSessionAsync = WebBrowser.openAuthSessionAsync as jest.Mock;

function whoopRow(consent_required: boolean, connected = false) {
  return {
    provider: 'whoop',
    display_name: 'WHOOP',
    requires_oauth: true,
    connected,
    needs_reauth: false,
    capabilities: ['sleep', 'recovery'],
    consent_required,
  };
}

describe('ConnectionsScreen — WHOOP owner authorization', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    initMobileOAuth.mockResolvedValue({ authorization_url: 'https://api.prod.whoop.com/oauth/oauth2/auth' });
    openAuthSessionAsync.mockReturnValue(new Promise(() => {}));
  });

  it('states the owner authorization first and starts WHOOP with its acceptance', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [whoopRow(true)] });
    render(<ConnectionsScreen />);
    fireEvent.press(await screen.findByText('Connect'));

    expect(await screen.findByText('Before you connect WHOOP')).toBeTruthy();
    expect(screen.getByText(/GDPR Article 20 and the EU Data Act/)).toBeTruthy();
    expect(screen.queryByText('whoop-setup:save')).toBeNull();

    // Continue is held until the box is ticked.
    expect(screen.getByTestId('provider-notice-continue')).toBeDisabled();

    fireEvent.press(screen.getByTestId('provider-notice-consent'));
    expect(screen.getByTestId('provider-notice-continue')).toBeEnabled();
    fireEvent.press(screen.getByTestId('provider-notice-continue'));
    fireEvent.press(await screen.findByText('whoop-setup:save'));

    await waitFor(() =>
      expect(initMobileOAuth).toHaveBeenCalledWith('whoop', 'dravr://oauth-callback', { tosConsent: true }),
    );
  });

  it('starts nothing when the notice is dismissed', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [whoopRow(true)] });
    render(<ConnectionsScreen />);
    fireEvent.press(await screen.findByText('Connect'));

    fireEvent.press(await screen.findByTestId('provider-notice-cancel'));

    await waitFor(() => expect(screen.queryByText('Before you connect WHOOP')).toBeNull());
    expect(screen.queryByText('whoop-setup:save')).toBeNull();
    expect(initMobileOAuth).not.toHaveBeenCalled();
  });

  it('goes straight to the WHOOP setup once the account has accepted the notice', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [whoopRow(false)] });
    render(<ConnectionsScreen />);
    fireEvent.press(await screen.findByText('Connect'));

    expect(screen.queryByText('Before you connect WHOOP')).toBeNull();
    fireEvent.press(await screen.findByText('whoop-setup:save'));

    await waitFor(() =>
      expect(initMobileOAuth).toHaveBeenCalledWith('whoop', 'dravr://oauth-callback', { tosConsent: false }),
    );
  });

  it('asks a connected WHOOP that owes the authorization to give it, then reconnects with it', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [whoopRow(true, true)] });
    render(<ConnectionsScreen />);

    expect(await screen.findByText('Authorize WHOOP to keep syncing')).toBeTruthy();
    expect(screen.queryByText('Connected')).toBeNull();

    fireEvent.press(screen.getByText('Authorize'));
    expect(await screen.findByText('Before you connect WHOOP')).toBeTruthy();
    fireEvent.press(screen.getByTestId('provider-notice-consent'));
    fireEvent.press(screen.getByTestId('provider-notice-continue'));
    fireEvent.press(await screen.findByText('whoop-setup:save'));

    await waitFor(() =>
      expect(initMobileOAuth).toHaveBeenCalledWith('whoop', 'dravr://oauth-callback', { tosConsent: true }),
    );
  });

  it('reads a connected WHOOP that gave it as Connected', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [whoopRow(false, true)] });
    render(<ConnectionsScreen />);

    expect(await screen.findByText('Connected')).toBeTruthy();
    expect(screen.queryByText('Authorize WHOOP to keep syncing')).toBeNull();
    expect(screen.queryByText('Authorize')).toBeNull();
  });
});
