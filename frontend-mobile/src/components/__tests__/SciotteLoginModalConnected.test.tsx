// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A connected Garmin sign-in ends the connect: onConnected takes no provider and no OAuth grant follows it
// ABOUTME: Red if the login, a 2FA pick or a code step hands a grant to authorize, even when the answer names one

import React from 'react';
import { render, screen, fireEvent, waitFor } from '@testing-library/react-native';
import { SciotteLoginModal } from '../SciotteLoginModal';
import { oauthApi } from '../../services/api';

jest.mock('expo-linking', () => ({ parse: jest.fn(), createURL: jest.fn() }));
jest.mock('../../utils/oauth', () => ({ getOAuthCallbackUrl: () => 'dravr://oauth-callback' }));
jest.mock('../../services/api', () => ({
  oauthApi: {
    sciotteLogin: jest.fn(),
    sciotteSelect2FA: jest.fn(),
    sciotteSubmitOTP: jest.fn(),
    sciotteCancel: jest.fn().mockResolvedValue({ cancelled: false }),
    initMobileOAuth: jest.fn(),
  },
}));
jest.mock('../OAuthAppSetupModal', () => ({ OAuthAppSetupModal: () => null }));

const sciotteLogin = oauthApi.sciotteLogin as jest.Mock;
const sciotteSelect2FA = oauthApi.sciotteSelect2FA as jest.Mock;
const sciotteSubmitOTP = oauthApi.sciotteSubmitOTP as jest.Mock;
const initMobileOAuth = oauthApi.initMobileOAuth as jest.Mock;

/**
 * A connected answer that still names Garmin's grant: the sheet must end the
 * connect all the same.
 */
const CONNECTED_NAMING_A_GRANT = {
  status: 'connected',
  provider: 'sciotte_garmin',
  oauth_follow_on: { provider: 'garmin' },
};

function signIn(): jest.Mock {
  const onConnected = jest.fn();
  render(<SciotteLoginModal visible onClose={jest.fn()} onConnected={onConnected} target="garmin" />);
  fireEvent.changeText(screen.getByTestId('sciotte-email'), 'athlete@example.test');
  fireEvent.changeText(screen.getByTestId('sciotte-password'), 'not-a-real-password');
  fireEvent.press(screen.getByTestId('sciotte-login-submit'));
  return onConnected;
}

/** onConnected ran once with no argument, and nothing started an authorization. */
async function endedConnected(onConnected: jest.Mock): Promise<void> {
  await waitFor(() => expect(onConnected).toHaveBeenCalledTimes(1));
  expect(onConnected.mock.calls).toEqual([[]]);
  expect(initMobileOAuth).not.toHaveBeenCalled();
}

describe('SciotteLoginModal (mobile) — a connected Garmin sign-in ends the connect', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('after the login', async () => {
    sciotteLogin.mockResolvedValue(CONNECTED_NAMING_A_GRANT);

    const onConnected = signIn();

    await endedConnected(onConnected);
  });

  it('after a 2FA pick', async () => {
    sciotteLogin.mockResolvedValue({ status: 'two_factor_choice', options: [{ id: 'sms', label: 'Text message' }] });
    sciotteSelect2FA.mockResolvedValue(CONNECTED_NAMING_A_GRANT);

    const onConnected = signIn();
    fireEvent.press(await screen.findByText('Text message'));

    await endedConnected(onConnected);
  });

  it('after the code step', async () => {
    sciotteLogin.mockResolvedValue({ status: 'otp_required' });
    sciotteSubmitOTP.mockResolvedValue(CONNECTED_NAMING_A_GRANT);

    const onConnected = signIn();
    fireEvent.changeText(await screen.findByTestId('sciotte-otp'), '246810');
    fireEvent.press(screen.getByTestId('sciotte-otp-submit'));

    await endedConnected(onConnected);
  });
});
