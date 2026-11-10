// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A sign-in-first card's account sign-in that cannot complete hands over to the provider's OAuth (carnet#1477)
// ABOUTME: Pins the reasons that fall back, the wrong credentials that never do, and a window offered no fallback

import React from 'react';
import { render, screen, fireEvent, waitFor } from '@testing-library/react-native';
import { i18n } from '@pierre/i18n';
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

function signIn(onFallBackToOAuth?: () => void): void {
  render(
    <SciotteLoginModal
      visible
      onClose={jest.fn()}
      onConnected={jest.fn()}
      target="garmin"
      onFallBackToOAuth={onFallBackToOAuth}
    />,
  );
  fireEvent.changeText(screen.getByTestId('sciotte-email'), 'athlete@example.test');
  fireEvent.changeText(screen.getByTestId('sciotte-password'), 'not-a-real-password');
  fireEvent.press(screen.getByTestId('sciotte-login-submit'));
}

const refusal = (reason: string) => ({ response: { status: 400, data: { details: { reason } } } });

describe('SciotteLoginModal (mobile) — the OAuth a failed sign-in falls back to', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it.each(['sign_in_unavailable', 'login_challenge'])(
    "hands a %s refusal to the provider's OAuth instead of showing it",
    async (reason) => {
      sciotteLogin.mockRejectedValue(refusal(reason));
      const onFallBackToOAuth = jest.fn();

      signIn(onFallBackToOAuth);

      await waitFor(() => expect(onFallBackToOAuth).toHaveBeenCalledTimes(1));
      expect(screen.queryByText(i18n.t('app.connectionFailed'))).toBeNull();
    },
  );

  it('keeps wrong credentials in the window, for the athlete to type again', async () => {
    sciotteLogin.mockResolvedValue({ status: 'failed', error: 'Invalid email or password' });
    const onFallBackToOAuth = jest.fn();

    signIn(onFallBackToOAuth);

    expect(await screen.findByText(i18n.t('app.connectionFailed'))).toBeTruthy();
    expect(onFallBackToOAuth).not.toHaveBeenCalled();
  });

  it('shows the failure when its card offers no OAuth to fall back to', async () => {
    sciotteLogin.mockRejectedValue(refusal('sign_in_unavailable'));

    signIn(undefined);

    expect(await screen.findByText(i18n.t('app.connectionFailed'))).toBeTruthy();
  });
});
