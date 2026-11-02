// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Closing the mobile sign-in sheet aborts the login it left running, so the server stops it too (carnet#1300)
// ABOUTME: Red if the request outlives the sheet, its abort reopens on an error, or the server is not told to cancel

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
    sciotteCancel: jest.fn(),
    initMobileOAuth: jest.fn(),
  },
}));
jest.mock('../OAuthAppSetupModal', () => ({ OAuthAppSetupModal: () => null }));

const sciotteLogin = oauthApi.sciotteLogin as jest.Mock;
const sciotteCancel = oauthApi.sciotteCancel as jest.Mock;

/** Type the credentials and log in. */
function logIn(): void {
  fireEvent.changeText(screen.getByTestId('sciotte-email'), 'athlete@example.test');
  fireEvent.changeText(screen.getByTestId('sciotte-password'), 'not-a-real-password');
  fireEvent.press(screen.getByTestId('sciotte-login-submit'));
}

/** The attempt id the `n`th login named. */
function attemptOf(n: number): string {
  return (sciotteLogin.mock.calls[n][1] as { attemptId: string }).attemptId;
}

/** A login the provider never answers: it settles only when its signal aborts, as axios does. */
function hangingLogin(_params: unknown, options: { signal?: AbortSignal } = {}) {
  return new Promise((_resolve, reject) => {
    options.signal?.addEventListener('abort', () => reject(new Error('canceled')), { once: true });
  });
}

function sheet(visible: boolean) {
  return (
    <SciotteLoginModal
      visible={visible}
      onClose={jest.fn()}
      onConnected={jest.fn()}
      target="garmin"
      consentRequired={false}
    />
  );
}

describe('SciotteLoginModal (mobile) — closing the sheet', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    sciotteLogin.mockImplementation(hangingLogin);
    sciotteCancel.mockResolvedValue({ cancelled: true });
  });

  it('cancels the attempt the login named, and names the next opening afresh', async () => {
    const { rerender } = render(sheet(true));
    logIn();
    await waitFor(() => expect(sciotteLogin).toHaveBeenCalledTimes(1));
    const first = attemptOf(0);
    expect(first).toMatch(/^[0-9a-f-]{36}$/);
    expect(sciotteCancel).not.toHaveBeenCalled();

    rerender(sheet(false));
    expect(sciotteCancel).toHaveBeenCalledTimes(1);
    expect(sciotteCancel).toHaveBeenCalledWith(first);

    rerender(sheet(true));
    await screen.findByTestId('sciotte-email');
    logIn();
    await waitFor(() => expect(sciotteLogin).toHaveBeenCalledTimes(2));
    expect(attemptOf(1)).not.toBe(first);
    rerender(sheet(false));
    expect(sciotteCancel.mock.calls).toEqual([[first], [attemptOf(1)]]);
  });

  it('cancels a flow parked on a code when the sheet closes', async () => {
    sciotteLogin.mockResolvedValue({ status: 'otp_required' });
    const { rerender } = render(sheet(true));
    logIn();
    await screen.findByTestId('sciotte-otp');

    rerender(sheet(false));

    expect(sciotteCancel).toHaveBeenCalledWith(attemptOf(0));
  });

  it('sends no cancel when nothing was in flight', () => {
    const { rerender, unmount } = render(sheet(true));
    rerender(sheet(false));
    unmount();

    expect(sciotteCancel).not.toHaveBeenCalled();
  });

  it('aborts the login in flight and reopens on a fresh form, not an error', async () => {
    const { rerender } = render(sheet(true));
    fireEvent.changeText(screen.getByTestId('sciotte-email'), 'athlete@example.test');
    fireEvent.changeText(screen.getByTestId('sciotte-password'), 'not-a-real-password');
    fireEvent.press(screen.getByTestId('sciotte-login-submit'));

    await waitFor(() => expect(sciotteLogin).toHaveBeenCalledTimes(1));
    const { signal } = sciotteLogin.mock.calls[0][1] as { signal: AbortSignal };
    expect(signal.aborted).toBe(false);

    rerender(sheet(false));
    expect(signal.aborted).toBe(true);

    rerender(sheet(true));
    expect(await screen.findByTestId('sciotte-email')).toHaveProp('value', '');
    expect(screen.queryByText(/connection failed/i)).toBeNull();
  });
});
