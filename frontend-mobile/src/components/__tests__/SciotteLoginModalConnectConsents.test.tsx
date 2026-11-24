// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mobile sign-in carries the consents the athlete ticked on the connect's sheet, by the credential login and on the phone
// ABOUTME: The window asks for none itself; a sign-in opened without an answer sends none (carnet#1157, carnet#1390)

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
    browserSigninCatalog: jest.fn(),
    browserSignin: jest.fn(),
  },
}));
jest.mock('../OAuthAppSetupModal', () => ({ OAuthAppSetupModal: () => null }));

const COOKIES = [{ name: 'SESSIONID', value: 'v', domain: 'connect.garmin.com', path: '/', secure: true, http_only: true }];

// The WebView lands signed in as soon as it shows: it hands its cookies over.
jest.mock('../auth/BrowserSigninWebView', () => {
  const actual = jest.requireActual('../auth/BrowserSigninWebView');
  const ReactActual = jest.requireActual('react');
  const { View } = jest.requireActual('react-native');
  return {
    ...actual,
    BrowserSigninWebView: ({ onSignedIn }: { onSignedIn: (cookies: unknown[]) => void }) => {
      ReactActual.useEffect(() => {
        onSignedIn([{ name: 'SESSIONID', value: 'v', domain: 'connect.garmin.com', path: '/', secure: true, http_only: true }]);
      }, [onSignedIn]);
      return ReactActual.createElement(View, { testID: 'browser-signin-webview' });
    },
  };
});

const sciotteLogin = oauthApi.sciotteLogin as jest.Mock;
const TICKED = ['garmin', 'garmin_workout_transfer'];

describe('SciotteLoginModal (mobile) — the consents ticked before the sign-in', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    sciotteLogin.mockResolvedValue({ status: 'failed', error: 'stop here' });
    (oauthApi.browserSigninCatalog as jest.Mock).mockResolvedValue({
      providers: [{ provider: 'garmin', login_url: 'https://sso.garmin.com/', success_patterns: ['/app'], cookie_domain: 'garmin.com' }],
    });
  });

  it('sends the ticked consents with the credential login, and shows no box of its own', async () => {
    render(<SciotteLoginModal visible onClose={jest.fn()} onConnected={jest.fn()} target="garmin" consents={TICKED} />);
    expect(screen.queryByTestId('connect-consent-continue')).toBeNull();

    fireEvent.changeText(screen.getByTestId('sciotte-email'), 'athlete@example.test');
    fireEvent.changeText(screen.getByTestId('sciotte-password'), 'not-a-real-password');
    fireEvent.press(screen.getByTestId('sciotte-login-submit'));

    await waitFor(() =>
      expect(sciotteLogin).toHaveBeenCalledWith({
        email: 'athlete@example.test',
        password: 'not-a-real-password',
        method: 'email',
        target: 'garmin',
        consents: TICKED,
      }, { signal: expect.any(AbortSignal), attemptId: expect.any(String) }),
    );
  });

  it('sends the empty answer when neither box was ticked', async () => {
    render(<SciotteLoginModal visible onClose={jest.fn()} onConnected={jest.fn()} target="garmin" consents={[]} />);
    fireEvent.changeText(screen.getByTestId('sciotte-email'), 'athlete@example.test');
    fireEvent.changeText(screen.getByTestId('sciotte-password'), 'not-a-real-password');
    fireEvent.press(screen.getByTestId('sciotte-login-submit'));

    await waitFor(() => expect(sciotteLogin).toHaveBeenCalledTimes(1));
    expect(sciotteLogin.mock.calls[0][0].consents).toEqual([]);
  });

  it('sends the ticked consents with the sign-in run on the phone', async () => {
    (oauthApi.browserSignin as jest.Mock).mockResolvedValue({ status: 'connected', provider: 'sciotte_garmin' });
    const onConnected = jest.fn();
    render(<SciotteLoginModal visible onClose={jest.fn()} onConnected={onConnected} target="garmin" consents={['garmin']} />);

    fireEvent.press(screen.getByTestId('sciotte-browser-signin'));

    await waitFor(() => expect(onConnected).toHaveBeenCalledTimes(1));
    expect(oauthApi.browserSignin).toHaveBeenCalledWith('garmin', COOKIES, {
      tosConsent: false,
      consents: ['garmin'],
      signal: expect.any(AbortSignal),
    });
  });
});
