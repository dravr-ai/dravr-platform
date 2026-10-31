// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Closing the mobile sign-in sheet aborts the login it left running, so the server stops it too (carnet#1300)
// ABOUTME: Red if the request outlives the sheet, or if its abort reopens the sheet on an error

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
    initMobileOAuth: jest.fn(),
  },
}));
jest.mock('../OAuthAppSetupModal', () => ({ OAuthAppSetupModal: () => null }));

const sciotteLogin = oauthApi.sciotteLogin as jest.Mock;

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
