// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A sign-in-first card's account sign-in that cannot complete hands over to the provider's OAuth (carnet#1477)
// ABOUTME: Pins the refusals that fall back, every other failure offering it beside a retry (Garmin and Strava), and a window offered none

import React from 'react';
import { render, screen, fireEvent, waitFor } from '@testing-library/react-native';
import { i18n } from '@pierre/i18n';
import { SciotteLoginModal } from '../SciotteLoginModal';
import { oauthApi } from '../../services/api';
import type { SciotteTarget } from '@pierre/shared-types';

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
const sciotteSubmitOTP = oauthApi.sciotteSubmitOTP as jest.Mock;

function signIn(onFallBackToOAuth?: () => void, target: SciotteTarget = 'garmin'): void {
  render(
    <SciotteLoginModal
      visible
      onClose={jest.fn()}
      onConnected={jest.fn()}
      target={target}
      onFallBackToOAuth={onFallBackToOAuth}
    />,
  );
  // A Strava window asks first how the athlete signs in: the email.
  if (target === 'strava') {
    fireEvent.press(screen.getByText(i18n.t('app.providerEmailLabel', { provider: 'Strava' })));
  }
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

  // An account the provider does not know by that email (one opened with
  // Google or Apple has no password there) answers like wrong credentials:
  // the athlete may retype, or connect the same account through OAuth.
  it('offers both a retry and the OAuth when the provider turns the credentials down', async () => {
    sciotteLogin.mockResolvedValue({ status: 'failed', error: "Couldn't find this account" });
    const onFallBackToOAuth = jest.fn();

    signIn(onFallBackToOAuth);

    expect(await screen.findByText(i18n.t('app.connectionFailed'))).toBeTruthy();
    expect(screen.getByText(i18n.t('app.tryAgain'))).toBeTruthy();
    expect(onFallBackToOAuth).not.toHaveBeenCalled();
    fireEvent.press(screen.getByTestId('sciotte-oauth-fallback'));
    expect(onFallBackToOAuth).toHaveBeenCalledTimes(1);
  });

  it('keeps a turned-down sign-in to a retry when its card has no OAuth', async () => {
    sciotteLogin.mockResolvedValue({ status: 'failed', error: 'Invalid email or password' });

    signIn(undefined);

    expect(await screen.findByText(i18n.t('app.connectionFailed'))).toBeTruthy();
    expect(screen.queryByTestId('sciotte-oauth-fallback')).toBeNull();
  });

  // A sign-in window never dead-ends: any other refusal keeps the athlete in
  // the window with its retry, and the card's OAuth beside it.
  it('keeps any other refusal in the window, offering the OAuth beside the retry', async () => {
    sciotteLogin.mockRejectedValue(refusal('login_flow_expired'));
    const onFallBackToOAuth = jest.fn();

    signIn(onFallBackToOAuth);

    expect(await screen.findByText(i18n.t('app.connectionFailed'))).toBeTruthy();
    expect(screen.getByText(i18n.t('app.tryAgain'))).toBeTruthy();
    expect(onFallBackToOAuth).not.toHaveBeenCalled();
    fireEvent.press(screen.getByTestId('sciotte-oauth-fallback'));
    expect(onFallBackToOAuth).toHaveBeenCalledTimes(1);
  });

  it('offers the OAuth beside the retry when the code step fails', async () => {
    sciotteLogin.mockResolvedValue({ status: 'otp_required' });
    sciotteSubmitOTP.mockRejectedValue(refusal('login_flow_expired'));
    const onFallBackToOAuth = jest.fn();

    signIn(onFallBackToOAuth);
    fireEvent.changeText(await screen.findByTestId('sciotte-otp'), '123456');
    fireEvent.press(screen.getByTestId('sciotte-otp-submit'));

    expect(await screen.findByText(i18n.t('app.connectionFailed'))).toBeTruthy();
    expect(screen.getByText(i18n.t('app.tryAgain'))).toBeTruthy();
    expect(screen.getByTestId('sciotte-oauth-fallback')).toBeTruthy();
    expect(onFallBackToOAuth).not.toHaveBeenCalled();
  });

  // The 2026-11-24 dead end: Strava turned an email sign-in down for an
  // account opened with Google or Apple. The window quotes Strava's reason,
  // points at those methods, and offers Strava's OAuth beside the retry.
  it('on a Strava card, quotes a turned-down email sign-in, advises Google or Apple and offers the OAuth', async () => {
    sciotteLogin.mockResolvedValue({ status: 'failed', error: "Couldn't find this account" });
    const onFallBackToOAuth = jest.fn();

    signIn(onFallBackToOAuth, 'strava');

    const reason = i18n.t('shell.sciotteTurnedDownReason', { provider: 'Strava', reason: "Couldn't find this account" });
    const advice = i18n.t('shell.sciotteTurnedDownAdviceMethods', { provider: 'Strava' });
    expect(advice).toMatch(/Google or Apple/);
    expect(await screen.findByText(`${reason} ${advice}`)).toBeTruthy();
    expect(screen.getByText(i18n.t('app.tryAgain'))).toBeTruthy();
    const offer = screen.getByTestId('sciotte-oauth-fallback');
    expect(offer).toHaveTextContent(i18n.t('app.continueWithProvider', { provider: 'Strava' }));
    fireEvent.press(offer);
    expect(onFallBackToOAuth).toHaveBeenCalledTimes(1);
  });

  it('shows the failure when its card offers no OAuth to fall back to', async () => {
    sciotteLogin.mockRejectedValue(refusal('sign_in_unavailable'));

    signIn(undefined);

    expect(await screen.findByText(i18n.t('app.connectionFailed'))).toBeTruthy();
  });
});
