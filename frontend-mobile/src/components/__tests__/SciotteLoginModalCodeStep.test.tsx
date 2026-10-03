// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mobile code step names the provider that sent the code and keeps the sign-in on a refused one
// ABOUTME: Pins the one-time-code input, the verifying state, the inline refusal, and the lapsed and failed sign-in copy

import React from 'react';
import { render, screen, fireEvent } from '@testing-library/react-native';
import { SciotteLoginModal } from '../SciotteLoginModal';
import { oauthApi } from '../../services/api';

jest.mock('expo-linking', () => ({ parse: jest.fn(), createURL: jest.fn() }));
jest.mock('../../utils/oauth', () => ({ getOAuthCallbackUrl: () => 'dravr://oauth-callback' }));
jest.mock('../../services/api', () => ({
  oauthApi: {
    sciotteLogin: jest.fn(),
    sciotteSelect2FA: jest.fn(),
    sciotteSubmitOTP: jest.fn(),
    initMobileOAuth: jest.fn(),
  },
}));
jest.mock('../OAuthAppSetupModal', () => ({ OAuthAppSetupModal: () => null }));

const sciotteLogin = oauthApi.sciotteLogin as jest.Mock;
const sciotteSubmitOTP = oauthApi.sciotteSubmitOTP as jest.Mock;
const sciotteSelect2FA = oauthApi.sciotteSelect2FA as jest.Mock;

const CODE_PROMPT = 'Enter the 6-digit code COROS sent you';
const LOGIN_COPY = 'This may take a moment while we securely connect your account';

/** Sign in to COROS up to its code step. */
async function reachCodeStep(onConnected = jest.fn()): Promise<jest.Mock> {
  render(<SciotteLoginModal visible onClose={jest.fn()} onConnected={onConnected} target="coros" />);
  signIn();
  await screen.findByTestId('sciotte-otp');
  return onConnected;
}

/** Type the credentials and log in. */
function signIn(): void {
  fireEvent.changeText(screen.getByTestId('sciotte-email'), 'athlete@example.test');
  fireEvent.changeText(screen.getByTestId('sciotte-password'), 'not-a-real-password');
  fireEvent.press(screen.getByTestId('sciotte-login-submit'));
}

async function submitCode(code: string): Promise<void> {
  fireEvent.changeText(screen.getByTestId('sciotte-otp'), code);
  fireEvent.press(screen.getByTestId('sciotte-otp-submit'));
}

describe('SciotteLoginModal (mobile) — the code step', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    sciotteLogin.mockResolvedValue({ status: 'otp_required' });
  });

  it('names the provider that sent the code, on a one-time-code input', async () => {
    await reachCodeStep();

    expect(screen.getByText(CODE_PROMPT)).toBeTruthy();
    expect(screen.queryByText(/authenticator/i)).toBeNull();
    const input = screen.getByTestId('sciotte-otp');
    expect(input.props.textContentType).toBe('oneTimeCode');
    expect(input.props.autoComplete).toBe('one-time-code');
    expect(input.props.maxLength).toBe(6);
    expect(input.props.accessibilityLabel).toBe(CODE_PROMPT);
    expect(screen.queryByTestId('sciotte-otp-error')).toBeNull();
  });

  it('says it is verifying the code while the code is checked', async () => {
    await reachCodeStep();
    sciotteSubmitOTP.mockReturnValueOnce(new Promise(() => {}));
    await submitCode('246810');

    expect(await screen.findByTestId('sciotte-verifying')).toBeTruthy();
    expect(screen.getAllByText('Verifying code...').length).toBeGreaterThan(0);
    expect(screen.queryByText(LOGIN_COPY)).toBeNull();
    expect(screen.queryByTestId('sciotte-otp')).toBeNull();
  });

  it('shows verifying, not the login copy, while a 2FA pick is checked', async () => {
    sciotteLogin.mockResolvedValueOnce({
      status: 'two_factor_choice',
      options: [{ id: 'sms', label: 'Text message' }],
    });
    render(<SciotteLoginModal visible onClose={jest.fn()} onConnected={jest.fn()} target="coros" />);
    signIn();
    sciotteSelect2FA.mockReturnValueOnce(new Promise(() => {}));
    fireEvent.press(await screen.findByText('Text message'));

    expect(await screen.findByTestId('sciotte-verifying')).toBeTruthy();
    expect(screen.getAllByText('Verifying...').length).toBeGreaterThan(0);
    expect(screen.queryByText(LOGIN_COPY)).toBeNull();
  });

  it('stays on the code step when a code is refused, then connects on the right one', async () => {
    const onConnected = await reachCodeStep();

    sciotteSubmitOTP.mockResolvedValueOnce({ status: 'otp_required', reason: 'code_rejected' });
    await submitCode('000000');

    const error = await screen.findByTestId('sciotte-otp-error');
    expect(error).toHaveTextContent("That code wasn't accepted. Check it and enter it again.");
    expect(error.props.accessibilityRole).toBe('alert');
    const input = screen.getByTestId('sciotte-otp');
    expect(input.props.value).toBe('');
    expect(input.props.autoFocus).toBe(true);

    sciotteSubmitOTP.mockResolvedValueOnce({ status: 'connected', provider: 'sciotte_coros' });
    await submitCode('246810');

    expect(await screen.findByText('COROS is ready to sync')).toBeTruthy();
    expect(onConnected).toHaveBeenCalledTimes(1);
    expect(sciotteSubmitOTP.mock.calls).toEqual([['000000'], ['246810']]);
  });

  it('asks to start again when the sign-in lapsed', async () => {
    await reachCodeStep();
    sciotteSubmitOTP.mockRejectedValueOnce({
      response: { status: 400, data: { details: { reason: 'login_flow_expired' } } },
    });
    await submitCode('246810');

    expect(
      await screen.findByText('This sign-in is no longer active. Please start the sign-in again.'),
    ).toBeTruthy();
  });

  it('words a failed code in the app, not in the provider prose', async () => {
    await reachCodeStep();
    sciotteSubmitOTP.mockResolvedValueOnce({ status: 'failed', error: 'Too many attempts' });
    await submitCode('246810');

    expect(await screen.findByText('That code did not work. Sign in again to get a new code.')).toBeTruthy();
    expect(screen.queryByText('Too many attempts')).toBeNull();
  });

  it('names no sender on a code step behind a Google sign-in', async () => {
    render(<SciotteLoginModal visible onClose={jest.fn()} onConnected={jest.fn()} target="strava" />);
    fireEvent.press(screen.getByText('Continue with Google'));
    signIn();
    const input = await screen.findByTestId('sciotte-otp');

    expect(screen.getByText('Enter the code sent to your device')).toBeTruthy();
    expect(input.props.accessibilityLabel).toBe('Enter the code sent to your device');
    expect(screen.queryByText(/Strava sent you/)).toBeNull();
  });

  it('words a failed 2FA pick in the app, not in the provider prose', async () => {
    sciotteLogin.mockResolvedValueOnce({
      status: 'two_factor_choice',
      options: [{ id: 'sms', label: 'Text message' }],
    });
    render(<SciotteLoginModal visible onClose={jest.fn()} onConnected={jest.fn()} target="coros" />);
    signIn();
    sciotteSelect2FA.mockResolvedValueOnce({ status: 'failed', error: 'Too many attempts' });
    fireEvent.press(await screen.findByText('Text message'));

    expect(await screen.findByText('Verification failed')).toBeTruthy();
    expect(screen.queryByText('Too many attempts')).toBeNull();
  });
});
