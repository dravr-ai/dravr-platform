// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the mobile ConnectionsScreen connect plans: OAuth while seats remain, the sign-in once the pool is full
// ABOUTME: A refused Strava grant falls back to its sign-in, a refused Garmin grant says so on its row; the history sign-in, export links, the move to OAuth

import React from 'react';
import { Alert } from 'react-native';
import { render, screen, fireEvent, waitFor, within } from '@testing-library/react-native';
import { i18n } from '@pierre/i18n';
import type { HistoryState } from '@pierre/shared-types';
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
const mockPush = jest.fn();
jest.mock('expo-router', () =>
  require('../../../../jest.expo-router').createExpoRouterMock({
    useRouter: () => ({ push: mockPush, replace: jest.fn(), back: jest.fn(), navigate: jest.fn(), canGoBack: () => true }),
  }),
);
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
      historyMode,
      onFallBackToOAuth,
    }: {
      visible: boolean;
      target: string;
      consentRequired?: boolean;
      historyMode?: boolean;
      onFallBackToOAuth?: () => void;
    }) =>
      visible
        ? React.createElement(
            React.Fragment,
            null,
            React.createElement(
              Text,
              null,
              `sciotte-modal:${target}${consentRequired ? ':consent' : ''}${historyMode ? ':history' : ''}`,
            ),
            // What a sign-in that could not complete does, when the card offers it.
            onFallBackToOAuth
              ? React.createElement(
                  Text,
                  { testID: 'sciotte-modal-unavailable', onPress: () => onFallBackToOAuth() },
                  'refuse:sign_in_unavailable',
                )
              : null,
          )
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
  describe('a sign-in-first card falls back to its OAuth (carnet#1477)', () => {
    const signInFirst = <T extends { seats_left: number }>(card: T) => ({ ...card, seats_left: 3 });

    it.each([
      ['strava', () => signInFirst(stravaCard('mirror'))],
      ['garmin', () => signInFirst(garminCard('mirror'))],
    ] as const)('starts the %s OAuth when its sign-in cannot complete', async (provider, card) => {
      getProvidersStatus.mockResolvedValue({ providers: [card()] });
      openAuthSessionAsync.mockReturnValue(new Promise(() => {}));

      render(<ConnectionsScreen />, { wrapper: queryWrapper() });
      fireEvent.press(await screen.findByText('Connect'));
      expect(await screen.findByText(`sciotte-modal:${provider}`)).toBeTruthy();
      expect(initMobileOAuth).not.toHaveBeenCalled();
      fireEvent.press(screen.getByTestId('sciotte-modal-unavailable'));

      await waitFor(() =>
        expect(initMobileOAuth).toHaveBeenCalledWith(provider, 'dravr://oauth-callback', { tosConsent: false }),
      );
      expect(screen.queryByText(`sciotte-modal:${provider}`)).toBeNull();
    });

    it('offers no fallback with the pool full', async () => {
      getProvidersStatus.mockResolvedValue({ providers: [stravaCard('mirror')] });

      render(<ConnectionsScreen />, { wrapper: queryWrapper() });
      fireEvent.press(await screen.findByText('Connect'));

      expect(await screen.findByText('sciotte-modal:strava')).toBeTruthy();
      expect(screen.queryByTestId('sciotte-modal-unavailable')).toBeNull();
    });

    it('never falls back from a sign-in a refused Strava grant opened', async () => {
      // Seats free and the sign-in first, yet the grant was started (the
      // athlete's tap on a stale row) and refused: its sign-in offers no way back.
      getProvidersStatus.mockResolvedValue({ providers: [stravaCard('oauth')] });
      openAuthSessionAsync.mockResolvedValue({ type: 'success', url: 'dravr://oauth-callback?success=false&error=cap_exceeded' });
      linkingParse.mockReturnValue({ queryParams: { success: 'false', error: 'cap_exceeded' } });

      render(<ConnectionsScreen />, { wrapper: queryWrapper() });
      getProvidersStatus.mockResolvedValue({ providers: [signInFirst(stravaCard('mirror'))] });
      fireEvent.press(await screen.findByText('Connect'));

      expect(await screen.findByText('sciotte-modal:strava')).toBeTruthy();
      expect(screen.queryByTestId('sciotte-modal-unavailable')).toBeNull();
    });
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

describe('ConnectionsScreen — a Garmin grant whose history is needed (carnet#1472)', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    initMobileOAuth.mockResolvedValue({ authorization_url: 'https://connect.garmin.com/oauth2Confirm' });
  });

  /** The Garmin row once its grant landed, its history at `state`. */
  function garminAt(state: HistoryState, archive_source: string | null = null) {
    return {
      ...garminCard('oauth'),
      connected: true,
      connected_backend: 'garmin',
      history: { state, sign_in_target: 'sciotte_garmin', archive_source },
    };
  }
  /** The Garmin row once its grant landed, its history needed or in. */
  function grantedGarmin(awaiting: boolean) {
    return garminAt(awaiting ? 'needed' : 'imported');
  }
  const line = (state: Exclude<HistoryState, 'not_applicable'>) =>
    i18n.t(`providers.history.${state}`, { provider: 'Garmin' });
  const addHint = i18n.t('providers.history.explainer', { provider: 'Garmin' });

  function grantComesBack() {
    openAuthSessionAsync.mockResolvedValue({ type: 'success', url: 'dravr://oauth-callback?success=true' });
    linkingParse.mockReturnValue({ queryParams: { success: 'true' } });
  }

  it('opens the Garmin sign-in by itself when the grant comes back with its history needed, with no sheet', async () => {
    getProvidersStatus
      .mockResolvedValueOnce({ providers: [garminCard('oauth')] })
      .mockResolvedValue({ providers: [grantedGarmin(true)] });
    grantComesBack();

    render(<ConnectionsScreen />, { wrapper: queryWrapper() });
    fireEvent.press(await screen.findByText('Connect'));

    expect(await screen.findByText('sciotte-modal:garmin:history')).toBeTruthy();
    expect(initMobileOAuth).toHaveBeenCalledWith('garmin', 'dravr://oauth-callback', { tosConsent: false });
  });

  it('stays on the grant once it holds activities', async () => {
    getProvidersStatus
      .mockResolvedValueOnce({ providers: [garminCard('oauth')] })
      .mockResolvedValue({ providers: [grantedGarmin(false)] });
    grantComesBack();

    render(<ConnectionsScreen />, { wrapper: queryWrapper() });
    fireEvent.press(await screen.findByText('Connect'));

    await waitFor(() => expect(getProvidersStatus).toHaveBeenCalledTimes(2));
    expect(screen.queryByText(/sciotte-modal:garmin/)).toBeNull();
  });

  it('says a refused Garmin grant did not connect, on its row, never the sign-in, and its tap tries again', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [garminCard('oauth')] });
    openAuthSessionAsync.mockResolvedValue({ type: 'success', url: 'dravr://oauth-callback?success=false&error=denied' });
    linkingParse.mockReturnValue({ queryParams: { success: 'false', error: 'denied' } });

    render(<ConnectionsScreen />, { wrapper: queryWrapper() });
    fireEvent.press(await screen.findByText('Connect'));

    const refused = await screen.findByTestId('provider-error-sciotte_garmin');
    expect(refused).toHaveTextContent(i18n.t('providers.connectRefused', { provider: 'Garmin' }));
    expect(screen.queryByText(/sciotte-modal:garmin/)).toBeNull();

    openAuthSessionAsync.mockReturnValue(new Promise(() => {}));
    fireEvent.press(refused);
    await waitFor(() => expect(initMobileOAuth).toHaveBeenCalledTimes(2));
    expect(initMobileOAuth).toHaveBeenLastCalledWith('garmin', 'dravr://oauth-callback', { tosConsent: false });
    expect(screen.queryByTestId('provider-error-sciotte_garmin')).toBeNull();
  });

  it('reads the rows again for a Garmin grant that came back refused for want of a seat, and signs in', async () => {
    getProvidersStatus
      .mockResolvedValueOnce({ providers: [garminCard('oauth')] })
      .mockResolvedValue({ providers: [garminCard('mirror')] });
    openAuthSessionAsync.mockResolvedValue({ type: 'success', url: 'dravr://oauth-callback?success=false&error=no_seat' });
    linkingParse.mockReturnValue({ queryParams: { success: 'false', error: 'no_seat' } });

    render(<ConnectionsScreen />, { wrapper: queryWrapper() });
    fireEvent.press(await screen.findByText('Connect'));

    expect(await screen.findByText('sciotte-modal:garmin')).toBeTruthy();
    expect(getProvidersStatus).toHaveBeenCalledTimes(2);
    expect(screen.queryByTestId('provider-error-sciotte_garmin')).toBeNull();
  });

  it('says "try again" for a Garmin grant refused for any other reason, reading nothing again', async () => {
    // A read after the refusal would sign in; a refusal that names no seat
    // reads nothing again, and the row offers its retry.
    getProvidersStatus
      .mockResolvedValueOnce({ providers: [garminCard('oauth')] })
      .mockResolvedValue({ providers: [garminCard('mirror')] });
    openAuthSessionAsync.mockResolvedValue({ type: 'success', url: 'dravr://oauth-callback?success=false&error=denied' });
    linkingParse.mockReturnValue({ queryParams: { success: 'false', error: 'denied' } });

    render(<ConnectionsScreen />, { wrapper: queryWrapper() });
    fireEvent.press(await screen.findByText('Connect'));

    expect(await screen.findByTestId('provider-error-sciotte_garmin')).toHaveTextContent(
      i18n.t('providers.connectRefused', { provider: 'Garmin' }),
    );
    expect(getProvidersStatus).toHaveBeenCalledTimes(1);
    expect(screen.queryByText(/sciotte-modal:garmin/)).toBeNull();
  });

  it('says a Garmin start that failed did not connect, on its row, never the sign-in', async () => {
    const consoleError = jest.spyOn(console, 'error').mockImplementation(() => undefined);
    getProvidersStatus.mockResolvedValue({ providers: [garminCard('oauth')] });
    initMobileOAuth.mockRejectedValue(new Error('network down'));

    render(<ConnectionsScreen />, { wrapper: queryWrapper() });
    fireEvent.press(await screen.findByText('Connect'));

    expect(await screen.findByTestId('provider-error-sciotte_garmin')).toBeTruthy();
    expect(screen.queryByText(/sciotte-modal:garmin/)).toBeNull();
    expect(getProvidersStatus).toHaveBeenCalledTimes(1);
    consoleError.mockRestore();
  });

  it('reads the rows again when the start was refused for want of a seat, and signs in once the pool is full', async () => {
    const consoleError = jest.spyOn(console, 'error').mockImplementation(() => undefined);
    getProvidersStatus
      .mockResolvedValueOnce({ providers: [garminCard('oauth')] })
      .mockResolvedValue({ providers: [garminCard('mirror')] });
    initMobileOAuth.mockRejectedValue({ response: { status: 403, data: { details: { reason: 'no_seat' } } } });

    render(<ConnectionsScreen />, { wrapper: queryWrapper() });
    fireEvent.press(await screen.findByText('Connect'));

    expect(await screen.findByText('sciotte-modal:garmin')).toBeTruthy();
    expect(getProvidersStatus).toHaveBeenCalledTimes(2);
    expect(screen.queryByTestId('provider-error-sciotte_garmin')).toBeNull();
    consoleError.mockRestore();
  });

  it('says the history is missing, and its action opens the Garmin sign-in directly', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [grantedGarmin(true)] });

    render(<ConnectionsScreen />, { wrapper: queryWrapper() });
    const history = await screen.findByTestId('provider-history-sciotte_garmin');
    expect(within(history).getByText(line('needed'))).toBeTruthy();
    expect(screen.getByText('Connected')).toBeTruthy();
    // The spoken name is the visible words; the hint naming the provider explains it.
    const add = screen.getByHintText(addHint);
    expect(add).toHaveTextContent(i18n.t('providers.history.add'));
    expect(add.props.accessibilityLabel).toBeUndefined();
    fireEvent.press(add);

    expect(await screen.findByText('sciotte-modal:garmin:history')).toBeTruthy();
    expect(initMobileOAuth).not.toHaveBeenCalled();
  });

  it('asks for the notice the grant card owes before the history sign-in', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [{ ...grantedGarmin(true), consent_required: true }] });

    render(<ConnectionsScreen />, { wrapper: queryWrapper() });
    fireEvent.press(await screen.findByHintText(addHint));

    expect(await screen.findByText('sciotte-modal:garmin:consent:history')).toBeTruthy();
  });

  it('confirms the history is in, and asks for nothing', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [garminAt('imported')] });

    render(<ConnectionsScreen />, { wrapper: queryWrapper() });
    const history = await screen.findByTestId('provider-history-sciotte_garmin');
    expect(within(history).getByText(line('imported'))).toBeTruthy();
    expect(screen.queryByText(i18n.t('providers.history.add'))).toBeNull();
    expect(screen.getByText('Disconnect')).toBeTruthy();
  });

  it('adds nothing to a row whose history is not applicable, or that carries none', async () => {
    // A row from a server that serves no `history` at all.
    const noHistory = { ...garminCard('oauth'), connected: true, connected_backend: 'garmin' };
    getProvidersStatus.mockResolvedValue({ providers: [garminAt('not_applicable')] });

    const first = render(<ConnectionsScreen />, { wrapper: queryWrapper() });
    expect(await screen.findByText('Connected')).toBeTruthy();
    expect(screen.queryByTestId('provider-history-sciotte_garmin')).toBeNull();
    expect(screen.queryByText(i18n.t('providers.history.add'))).toBeNull();
    first.unmount();

    getProvidersStatus.mockResolvedValue({ providers: [noHistory] });
    render(<ConnectionsScreen />, { wrapper: queryWrapper() });
    expect(await screen.findByText('Connected')).toBeTruthy();
    expect(screen.queryByTestId('provider-history-sciotte_garmin')).toBeNull();
  });

  it('reconnects a flagged Garmin grant through the Garmin sign-in, never Garmin OAuth (carnet#1494)', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [{ ...grantedGarmin(false), needs_reauth: true }] });

    render(<ConnectionsScreen />, { wrapper: queryWrapper() });
    expect(await screen.findByText('Expired')).toBeTruthy();
    fireEvent.press(screen.getByText('Reconnect'));

    expect(await screen.findByText('sciotte-modal:garmin')).toBeTruthy();
    expect(initMobileOAuth).not.toHaveBeenCalled();
  });
});

describe('ConnectionsScreen — export links', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('links a card to its export import, on its source', async () => {
    getProvidersStatus.mockResolvedValue({
      providers: [
        {
          ...garminCard('oauth'),
          connected: true,
          connected_backend: 'garmin',
          history: { state: 'imported', sign_in_target: 'sciotte_garmin', archive_source: 'garmin_archive' },
        },
      ],
    });

    render(<ConnectionsScreen />, { wrapper: queryWrapper() });
    const link = await screen.findByTestId('provider-export-sciotte_garmin');
    expect(link).toHaveTextContent(i18n.t('providers.import.cardLink'));
    fireEvent.press(link);

    expect(mockPush).toHaveBeenCalledWith({ pathname: '/(app)/import', params: { source: 'garmin_archive' } });
  });

  it('links a Strava card not yet connected to the Strava export', async () => {
    getProvidersStatus.mockResolvedValue({
      providers: [{ ...stravaCard('oauth'), history: { state: 'not_applicable', sign_in_target: null, archive_source: 'strava_archive' } }],
    });

    render(<ConnectionsScreen />, { wrapper: queryWrapper() });
    fireEvent.press(await screen.findByTestId('provider-export-sciotte'));

    expect(mockPush).toHaveBeenCalledWith({ pathname: '/(app)/import', params: { source: 'strava_archive' } });
    expect(screen.getByText('Connect')).toBeTruthy();
  });

  it('shows no export link when the server offers no source for the card', async () => {
    getProvidersStatus.mockResolvedValue({
      providers: [{ ...stravaCard('oauth'), history: { state: 'not_applicable', sign_in_target: null, archive_source: null } }],
    });

    render(<ConnectionsScreen />, { wrapper: queryWrapper() });
    expect(await screen.findByText('Connect')).toBeTruthy();
    expect(screen.queryByTestId('provider-export-sciotte')).toBeNull();
  });
});

describe('ConnectionsScreen — a Garmin reconnect signs in (carnet#1494)', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    initMobileOAuth.mockResolvedValue({ authorization_url: 'https://connect.garmin.com/oauth2Confirm' });
    openAuthSessionAsync.mockReturnValue(new Promise(() => {}));
  });

  /** A connected Garmin row, served by `backend`, while a seat is free on Garmin's OAuth. */
  const connectedGarmin = (backend: string, overrides: Record<string, unknown> = {}) => ({
    ...garminCard('oauth'),
    connected: true,
    connected_backend: backend,
    ...overrides,
  });

  it('reconnects an expired sign-in or a dead grant through the Garmin sign-in, never Garmin OAuth', async () => {
    for (const backend of ['sciotte_garmin', 'garmin']) {
      getProvidersStatus.mockResolvedValue({
        providers: [connectedGarmin(backend, { needs_reauth: true, recommended_backend: 'mirror' })],
      });

      const view = render(<ConnectionsScreen />, { wrapper: queryWrapper() });
      fireEvent.press(await screen.findByText('Reconnect'));

      expect(await screen.findByText('sciotte-modal:garmin')).toBeTruthy();
      expect(initMobileOAuth).not.toHaveBeenCalled();
      view.unmount();
    }
  });

  it('keeps a sign-in-only Garmin row connected, asking nothing, though a seat is free', async () => {
    getProvidersStatus.mockResolvedValue({ providers: [connectedGarmin('sciotte_garmin')] });

    render(<ConnectionsScreen />, { wrapper: queryWrapper() });
    expect(await screen.findByText('Connected')).toBeTruthy();
    expect(screen.queryByText(i18n.t('app.connectProvider', { provider: 'Garmin' }))).toBeNull();
  });
});
