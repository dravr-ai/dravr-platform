// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the Connections rows — a brand glyph per provider, one status word and one ink action per state, no pill and no filled button
// ABOUTME: And the row's behaviour: disconnect confirms, long-press opens the platform menu, the connected-apps section pushes its screen

import React from 'react';
import { ActionSheetIOS, Alert, Platform } from 'react-native';
import { fireEvent, render, screen, waitFor } from '@testing-library/react-native';
import { ConnectionsScreen } from '../ConnectionsScreen';
import { InitialsAvatar } from '../../../components/ui';
import { oauthApi } from '../../../services/api';
import type { ExtendedProviderStatus } from '../../../types';
import { networkFailure } from '../../../../integration/app/helpers/apiRefusal';

const mockPush = jest.fn();

jest.mock('expo-web-browser', () => ({
  openAuthSessionAsync: jest.fn(() => new Promise(() => {})),
}));
jest.mock('expo-linking', () => ({
  parse: jest.fn(),
}));
jest.mock('expo-router', () => ({
  useRouter: () => ({ push: mockPush, back: jest.fn() }),
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
jest.mock('../../../components/IntervalsIcuLinkModal', () => ({ IntervalsIcuLinkModal: () => null }));
jest.mock('../../../components/OAuthCredentialsSection', () => ({ OAuthCredentialsSection: () => null }));
jest.mock('../../../components/OAuthAppSetupModal', () => ({ OAuthAppSetupModal: () => null }));
// The glyph as a text node carrying its provider id, so a spec can find the
// mark a row drew without rendering SVG.
jest.mock('../../../components/icons/BrandIcons', () => {
  const React = require('react');
  const { Text } = require('react-native');
  const known = ['sciotte', 'strava', 'sciotte_garmin', 'garmin', 'sciotte_trainingpeaks', 'whoop', 'intervals_icu'];
  return {
    providerGlyph: (providerId: string) =>
      known.includes(providerId)
        ? () => React.createElement(Text, { testID: `glyph-${providerId}` }, providerId)
        : null,
  };
});
jest.mock('@expo/vector-icons', () => {
  const React = require('react');
  const { Text } = require('react-native');
  return {
    Feather: ({ name }: { name: string }) => React.createElement(Text, { testID: `icon-${name}` }, name),
  };
});

const getProvidersStatus = oauthApi.getProvidersStatus as jest.Mock;
const initMobileOAuth = oauthApi.initMobileOAuth as jest.Mock;

function provider(
  id: string,
  displayName: string,
  overrides: Partial<ExtendedProviderStatus> = {},
): ExtendedProviderStatus {
  return {
    provider: id,
    display_name: displayName,
    requires_oauth: false,
    connected: false,
    needs_reauth: false,
    capabilities: ['activities'],
    consent_required: false,
    ...overrides,
  };
}

const disconnectedStrava = provider('sciotte', 'Strava', { recommended_backend: 'oauth', seats_left: 3 });
const connectedWhoop = provider('whoop', 'WHOOP', { requires_oauth: true, connected: true });
const expiredGarmin = provider('garmin', 'Garmin', { requires_oauth: true, connected: true, needs_reauth: true });

type Instance = { type: unknown; props: { className?: string } };

/** Every className under an element, one per host view — the composite wrapper above each host repeats it. */
function classNamesUnder(root: ReturnType<typeof screen.getByTestId>): string[] {
  return root
    .findAll((node: Instance) => typeof node.type === 'string' && typeof node.props.className === 'string')
    .map((node: Instance) => node.props.className as string);
}

async function renderWith(providers: ExtendedProviderStatus[]) {
  getProvidersStatus.mockResolvedValue({ providers });
  render(<ConnectionsScreen />);
  await waitFor(() => expect(screen.getByTestId(`provider-row-${providers[0].provider}`)).toBeTruthy());
}

describe('ConnectionsScreen rows', () => {
  const originalOS = Platform.OS;

  beforeEach(() => {
    jest.clearAllMocks();
    jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
    jest.spyOn(ActionSheetIOS, 'showActionSheetWithOptions').mockImplementation(() => undefined);
    initMobileOAuth.mockResolvedValue({ authorization_url: 'https://example.test/authorize' });
  });

  afterEach(() => {
    (Platform as { OS: string }).OS = originalOS;
    jest.restoreAllMocks();
  });

  it('draws the brand glyph of every known provider and an initials circle for an unknown one', async () => {
    await renderWith([
      disconnectedStrava,
      provider('sciotte_garmin', 'Garmin'),
      provider('sciotte_trainingpeaks', 'TrainingPeaks', { consent_required: true }),
      provider('whoop', 'WHOOP', { requires_oauth: true }),
      provider('intervals_icu', 'Intervals.icu'),
      provider('polar', 'Polar Flow', { requires_oauth: true }),
    ]);

    for (const id of ['sciotte', 'sciotte_garmin', 'sciotte_trainingpeaks', 'whoop', 'intervals_icu']) {
      expect(screen.getByTestId(`glyph-${id}`)).toBeTruthy();
    }
    expect(screen.queryByTestId('glyph-polar')).toBeNull();
    const polarRow = screen.getByTestId('provider-row-polar');
    expect(polarRow.findAllByType(InitialsAvatar)).toHaveLength(1);
  });

  it('hides the bare strava row the Strava-branded sciotte row stands in for', async () => {
    await renderWith([disconnectedStrava, provider('strava', 'Strava', { requires_oauth: true })]);
    expect(screen.queryByTestId('provider-row-strava')).toBeNull();
  });

  it('a disconnected provider carries the ink "Connect" and no status word', async () => {
    await renderWith([disconnectedStrava]);

    const action = screen.getByTestId('provider-action-sciotte');
    expect(action.props.children).toBe('Connect');
    expect(action.props.className).toContain('text-primary');
    expect(screen.queryByText('Connected')).toBeNull();
    expect(screen.queryByText('Disconnect')).toBeNull();
  });

  it('a connected provider carries the dot, "Connected" and the ink "Disconnect"', async () => {
    await renderWith([connectedWhoop]);

    const row = screen.getByTestId('provider-row-whoop');
    const classes = classNamesUnder(row);
    expect(classes.filter((c) => c === 'w-2 h-2 rounded-full bg-success')).toHaveLength(1);
    expect(screen.getByText('Connected').props.className).toContain('text-text-secondary');
    expect(screen.getByTestId('provider-action-whoop').props.children).toBe('Disconnect');
  });

  it('an expired provider carries "Expired" in the warning ink and the ink "Reconnect", which restarts OAuth', async () => {
    await renderWith([expiredGarmin]);

    expect(screen.getByText('Expired').props.className).toContain('text-warning');
    const action = screen.getByTestId('provider-action-garmin');
    expect(action.props.children).toBe('Reconnect');
    fireEvent.press(action);
    await waitFor(() => expect(initMobileOAuth).toHaveBeenCalledWith('garmin', 'dravr://oauth-callback'));
    expect(screen.queryByText('Connected')).toBeNull();
  });

  it('pressing "Disconnect" asks for confirmation naming the provider', async () => {
    await renderWith([connectedWhoop]);

    fireEvent.press(screen.getByTestId('provider-action-whoop'));

    expect(Alert.alert).toHaveBeenCalledTimes(1);
    const [title, message, buttons] = (Alert.alert as jest.Mock).mock.calls[0] as [
      string,
      string,
      Array<{ text: string; style?: string }>,
    ];
    expect(title).toContain('WHOOP');
    expect(message).toContain('WHOOP');
    expect(buttons.map((b) => b.text)).toEqual(['Cancel', 'Disconnect']);
  });

  it('a long-press on a connected row presents the platform menu under the provider name', async () => {
    (Platform as { OS: string }).OS = 'ios';
    await renderWith([connectedWhoop, expiredGarmin]);

    fireEvent(screen.getByTestId('provider-row-whoop'), 'longPress');
    const spy = ActionSheetIOS.showActionSheetWithOptions as unknown as jest.Mock;
    expect(spy).toHaveBeenCalledTimes(1);
    expect(spy.mock.calls[0][0]).toMatchObject({ title: 'WHOOP', options: ['Disconnect', 'Cancel'] });

    fireEvent(screen.getByTestId('provider-row-garmin'), 'longPress');
    expect(spy).toHaveBeenCalledTimes(2);
    expect(spy.mock.calls[1][0]).toMatchObject({ title: 'Garmin', options: ['Reconnect', 'Disconnect', 'Cancel'] });
  });

  it('a disconnected row has no menu to long-press', async () => {
    (Platform as { OS: string }).OS = 'ios';
    await renderWith([disconnectedStrava]);

    fireEvent(screen.getByTestId('provider-row-sciotte'), 'longPress');
    expect(ActionSheetIOS.showActionSheetWithOptions).not.toHaveBeenCalled();
  });

  it('draws no pill and no filled button in a provider row — the 8 px dot is the only circle', async () => {
    await renderWith([disconnectedStrava, connectedWhoop, expiredGarmin]);

    for (const id of ['sciotte', 'whoop', 'garmin']) {
      const classes = classNamesUnder(screen.getByTestId(`provider-row-${id}`));
      expect(classes.some((c) => /\bbg-primary\b/.test(c))).toBe(false);
      const circles = classes.filter((c) => c.includes('rounded-full'));
      expect(circles.every((c) => c === 'w-2 h-2 rounded-full bg-success')).toBe(true);
    }
  });

  it('the connected-apps section is one sentence and one ink action that opens the connected-apps screen', async () => {
    await renderWith([disconnectedStrava]);

    expect(screen.getByText('No connected apps yet')).toBeTruthy();
    fireEvent.press(screen.getByTestId('connections-manage-apps'));
    expect(mockPush).toHaveBeenCalledWith('/(app)/(tabs)/(settings)/connected-apps');
  });

  it('a failed load is one error line with an inline retry that reloads', async () => {
    getProvidersStatus.mockRejectedValueOnce(networkFailure());
    const consoleSpy = jest.spyOn(console, 'error').mockImplementation(() => undefined);
    render(<ConnectionsScreen />);

    const retry = await screen.findByTestId('connections-retry');
    expect(screen.getByText('Network error. Check your connection.').props.className).toContain('text-error');
    expect(retry.props.className).toContain('text-primary');

    getProvidersStatus.mockResolvedValueOnce({ providers: [disconnectedStrava] });
    fireEvent.press(retry);
    await waitFor(() => expect(screen.getByTestId('provider-row-sciotte')).toBeTruthy());
    expect(getProvidersStatus).toHaveBeenCalledTimes(2);
    consoleSpy.mockRestore();
  });
});
