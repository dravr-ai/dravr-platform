// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A Garmin sign-in hands the grant it still owes to the caller, which authorizes it right after (carnet#1494)
// ABOUTME: Pins the follow-on the server names reaching onConnected, and a sign-in that owes nothing passing none

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

function signIn(onConnected: (followOn?: string) => void): void {
  render(<SciotteLoginModal visible onClose={jest.fn()} onConnected={onConnected} target="garmin" />);
  fireEvent.changeText(screen.getByTestId('sciotte-email'), 'athlete@example.test');
  fireEvent.changeText(screen.getByTestId('sciotte-password'), 'not-a-real-password');
  fireEvent.press(screen.getByTestId('sciotte-login-submit'));
}

describe('SciotteLoginModal (mobile) — the grant a Garmin sign-in goes on to', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('hands the owed grant to the caller, which authorizes it right after', async () => {
    sciotteLogin.mockResolvedValue({
      status: 'connected',
      provider: 'sciotte_garmin',
      oauth_follow_on: { provider: 'garmin' },
    });
    const onConnected = jest.fn();

    signIn(onConnected);

    await waitFor(() => expect(onConnected).toHaveBeenCalledWith('garmin'));
  });

  it('passes no follow-on when the sign-in owes nothing', async () => {
    sciotteLogin.mockResolvedValue({ status: 'connected', provider: 'sciotte_garmin', oauth_follow_on: null });
    const onConnected = jest.fn();

    signIn(onConnected);

    await waitFor(() => expect(onConnected).toHaveBeenCalledTimes(1));
    expect(onConnected).toHaveBeenCalledWith(undefined);
  });
});
