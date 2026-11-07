// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Connections pane's Intervals.icu row — the authorization while the server offers it, the key form otherwise
// ABOUTME: Pins that a key-linked row reconnects through the key form and an authorized row straight through the authorization

import React from 'react';
import { render, screen, fireEvent, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ExtendedProviderStatus } from '@pierre/shared-types';
import { ConnectionsScreen } from '../ConnectionsScreen';
import { oauthApi } from '../../../services/api';
import * as WebBrowser from 'expo-web-browser';

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
jest.mock('../../../components/SciotteLoginModal', () => ({ SciotteLoginModal: () => null }));
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
jest.mock('../../../components/OAuthCredentialsSection', () => ({ OAuthCredentialsSection: () => null }));
jest.mock('../../../components/OAuthAppSetupModal', () => {
  const React = require('react');
  const { Text } = require('react-native');
  return {
    OAuthAppSetupModal: ({ visible }: { visible: boolean }) =>
      visible ? React.createElement(Text, null, 'own-app-setup') : null,
  };
});

const getProvidersStatus = oauthApi.getProvidersStatus as jest.Mock;
const initMobileOAuth = oauthApi.initMobileOAuth as jest.Mock;
const openAuthSessionAsync = WebBrowser.openAuthSessionAsync as jest.Mock;

function intervalsCard(overrides: Partial<ExtendedProviderStatus> = {}): ExtendedProviderStatus {
  return {
    provider: 'intervals_icu',
    display_name: 'Intervals.icu',
    description: '',
    requires_oauth: true,
    connected: false,
    needs_reauth: false,
    capabilities: ['activities', 'health'],
    consent_required: false,
    own_app_required: false,
    oauth_callback_url: 'https://app.dravr.ai/api/oauth/callback/intervals_icu',
    ...overrides,
  };
}

/** The screen's consent action is a React Query mutation, so it renders under a client. */
function queryWrapper() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return ({ children }: { children: React.ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
}

describe('ConnectionsScreen — Intervals.icu', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    initMobileOAuth.mockResolvedValue({ authorization_url: 'https://intervals.icu/oauth/authorize' });
    // Keep the browser session pending so only the launch is asserted.
    openAuthSessionAsync.mockReturnValue(new Promise(() => {}));
  });

  it('opens the sheet leading with the authorization, and starts it from there', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [intervalsCard()] });
    render(<ConnectionsScreen />, { wrapper: queryWrapper() });

    fireEvent.press(await screen.findByTestId('provider-action-intervals_icu'));
    expect(await screen.findByText('intervals-sheet:authorize')).toBeTruthy();
    expect(initMobileOAuth).not.toHaveBeenCalled();

    fireEvent.press(screen.getByTestId('intervals-sheet'));

    await waitFor(() =>
      expect(initMobileOAuth).toHaveBeenCalledWith('intervals_icu', 'dravr://oauth-callback', { tosConsent: false }),
    );
  });

  it('opens the key form alone, never an own-app setup, while no client resolves', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [intervalsCard({ own_app_required: true })] });
    render(<ConnectionsScreen />, { wrapper: queryWrapper() });

    fireEvent.press(await screen.findByTestId('provider-action-intervals_icu'));

    expect(await screen.findByText('intervals-sheet:key')).toBeTruthy();
    expect(screen.queryByText('own-app-setup')).toBeNull();
    expect(initMobileOAuth).not.toHaveBeenCalled();
  });

  it('reconnects a key-linked row through the key form, never the authorization', async () => {
    getProvidersStatus.mockResolvedValue({
      providers: [intervalsCard({ connected: true, needs_reauth: true, connection_type: 'manual' })],
    });
    render(<ConnectionsScreen />, { wrapper: queryWrapper() });

    fireEvent.press(await screen.findByTestId('provider-action-intervals_icu'));

    expect(await screen.findByText('intervals-sheet:key')).toBeTruthy();
    expect(initMobileOAuth).not.toHaveBeenCalled();
  });

  it('reconnects an authorized row straight through the authorization', async () => {
    getProvidersStatus.mockResolvedValue({
      providers: [intervalsCard({ connected: true, needs_reauth: true, connection_type: 'oauth' })],
    });
    render(<ConnectionsScreen />, { wrapper: queryWrapper() });

    fireEvent.press(await screen.findByTestId('provider-action-intervals_icu'));

    await waitFor(() =>
      expect(initMobileOAuth).toHaveBeenCalledWith('intervals_icu', 'dravr://oauth-callback', { tosConsent: false }),
    );
    expect(screen.queryByTestId('intervals-sheet')).toBeNull();
  });
});
