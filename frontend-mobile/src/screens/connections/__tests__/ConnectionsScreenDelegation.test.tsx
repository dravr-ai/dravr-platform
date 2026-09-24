// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the TrainingPeaks row when a group coach serves it — "Connected through", the pending link, the coach account
// ABOUTME: A delegated row's one action and its long-press menu both read "Unlink", since disconnecting ends the coach's link

import React from 'react';
import { ActionSheetIOS, Alert, Platform } from 'react-native';
import { fireEvent, render, screen, waitFor } from '@testing-library/react-native';
import { ConnectionsScreen } from '../ConnectionsScreen';
import { oauthApi } from '../../../services/api';
import type { ExtendedProviderStatus, ProviderDelegation } from '@pierre/shared-types';

jest.mock('expo-web-browser', () => ({ openAuthSessionAsync: jest.fn(() => new Promise(() => {})) }));
jest.mock('expo-linking', () => ({ parse: jest.fn() }));
jest.mock('expo-router', () => ({ useRouter: () => ({ push: jest.fn(), back: jest.fn() }) }));
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
jest.mock('../../../components/OAuthAppSetupModal', () => ({ OAuthAppSetupModal: () => null }));
jest.mock('../../../components/icons/BrandIcons', () => ({ providerGlyph: () => null }));
jest.mock('@expo/vector-icons', () => {
  const React = require('react');
  const { Text } = require('react-native');
  return { Feather: ({ name }: { name: string }) => React.createElement(Text, null, name) };
});

const getProvidersStatus = oauthApi.getProvidersStatus as jest.Mock;
const ID = 'sciotte_trainingpeaks';

function delegation(status: 'proposed' | 'confirmed', coachNeedsReauth = false): ProviderDelegation {
  return {
    connection_id: 'dc-1',
    group_id: 'group-1',
    group_name: 'Harricana 2027',
    coach_display_name: 'Casey Coach',
    status,
    coach_needs_reauth: coachNeedsReauth,
  };
}

function trainingPeaks(overrides: Partial<ExtendedProviderStatus>): ExtendedProviderStatus {
  return {
    provider: ID,
    display_name: 'TrainingPeaks',
    requires_oauth: false,
    connected: false,
    needs_reauth: false,
    capabilities: ['activities'],
    consent_required: false,
    ...overrides,
  };
}

async function renderWith(row: ExtendedProviderStatus) {
  getProvidersStatus.mockResolvedValue({ providers: [row] });
  render(<ConnectionsScreen />);
  await waitFor(() => expect(screen.getByTestId(`provider-row-${ID}`)).toBeTruthy());
}

describe('ConnectionsScreen — TrainingPeaks through a coach', () => {
  const originalOS = Platform.OS;

  beforeEach(() => {
    jest.clearAllMocks();
    jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
    jest.spyOn(ActionSheetIOS, 'showActionSheetWithOptions').mockImplementation(() => undefined);
  });

  afterEach(() => {
    (Platform as { OS: string }).OS = originalOS;
    jest.restoreAllMocks();
  });

  it('reads "Connected through" the coach and offers Unlink', async () => {
    await renderWith(trainingPeaks({ connected: true, delegation: delegation('confirmed') }));

    expect(screen.getByTestId(`provider-subtitle-${ID}`)).toHaveTextContent('Connected through Casey Coach');
    expect(screen.getByTestId(`provider-action-${ID}`).props.children).toBe('Unlink');
    expect(screen.queryByText('Connected')).toBeNull();
  });

  it('says the coach must reconnect, not the athlete', async () => {
    await renderWith(trainingPeaks({ connected: true, delegation: delegation('confirmed', true) }));

    expect(screen.getByTestId(`provider-subtitle-${ID}`)).toHaveTextContent(
      'Casey Coach needs to reconnect TrainingPeaks; your workouts are paused until then.',
    );
    expect(screen.queryByText('Expired')).toBeNull();
  });

  it('labels the long-press menu row Unlink for a delegated connection', async () => {
    (Platform as { OS: string }).OS = 'ios';
    await renderWith(trainingPeaks({ connected: true, delegation: delegation('confirmed') }));

    fireEvent(screen.getByTestId(`provider-row-${ID}`), 'longPress');
    const spy = ActionSheetIOS.showActionSheetWithOptions as unknown as jest.Mock;
    expect(spy.mock.calls[0][0]).toMatchObject({ title: 'TrainingPeaks', options: ['Unlink', 'Cancel'] });
  });

  it('points a pending link at the group it waits in', async () => {
    await renderWith(trainingPeaks({ delegation: delegation('proposed') }));

    expect(screen.getByTestId(`provider-subtitle-${ID}`)).toHaveTextContent(
      'Casey Coach asked to link your workouts — review it in Harricana 2027',
    );
  });

  it('marks a coach account and says where its athletes are linked', async () => {
    await renderWith(trainingPeaks({ connected: true, account_role: 'coach' }));

    expect(screen.getByTestId(`provider-coach-account-${ID}`)).toHaveTextContent('Coach account');
    expect(screen.getByTestId(`provider-subtitle-${ID}`)).toHaveTextContent(
      'TrainingPeaks keeps no calendar for a coach account. Link your athletes from a group you coach.',
    );
  });
});
