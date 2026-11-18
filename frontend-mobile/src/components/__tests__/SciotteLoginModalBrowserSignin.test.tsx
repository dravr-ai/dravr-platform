// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: carnet#1617 — on the phone, a provider sign-in runs in a WebView of the provider's own page, and its cookies connect it
// ABOUTME: Pins the button that opens it, the hand-over that connects, and the cookie read that keeps only the provider's domain

import React from 'react';
import { render, screen, fireEvent, waitFor } from '@testing-library/react-native';
import CookieManager from '@react-native-cookies/cookies';
import { SciotteLoginModal } from '../SciotteLoginModal';
import { readProviderCookies } from '../auth/BrowserSigninWebView';
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

// The WebView lands signed in as soon as it shows: it hands its cookies over,
// unless a test holds the hand-over back to play it late.
const webView: { handsOver: boolean; onSignedIn: ((cookies: unknown[]) => void) | null } = {
  handsOver: true,
  onSignedIn: null,
};
jest.mock('../auth/BrowserSigninWebView', () => {
  const actual = jest.requireActual('../auth/BrowserSigninWebView');
  const ReactActual = jest.requireActual('react');
  const { View } = jest.requireActual('react-native');
  return {
    ...actual,
    BrowserSigninWebView: ({ onSignedIn }: { onSignedIn: (cookies: unknown[]) => void }) => {
      ReactActual.useEffect(() => {
        webView.onSignedIn = onSignedIn;
        if (!webView.handsOver) return;
        onSignedIn([{ name: 'SESSIONID', value: 'v', domain: 'connect.garmin.com', path: '/', secure: true, http_only: true }]);
      }, [onSignedIn]);
      return ReactActual.createElement(View, { testID: 'browser-signin-webview' });
    },
  };
});

const GARMIN = {
  provider: 'garmin',
  login_url: 'https://sso.garmin.com/portal/sso/en-US/sign-in',
  success_patterns: ['/app'],
  cookie_domain: 'garmin.com',
};

describe('SciotteLoginModal (mobile) — a sign-in on the phone itself', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    webView.handsOver = true;
    webView.onSignedIn = null;
    (oauthApi.browserSigninCatalog as jest.Mock).mockResolvedValue({ providers: [GARMIN] });
  });

  it('connects the provider with the cookies its own page left on the phone', async () => {
    (oauthApi.browserSignin as jest.Mock).mockResolvedValue({ status: 'connected', provider: 'sciotte_garmin' });
    const onConnected = jest.fn();
    render(<SciotteLoginModal visible onClose={jest.fn()} onConnected={onConnected} target="garmin" />);

    fireEvent.press(screen.getByTestId('sciotte-browser-signin'));

    await waitFor(() => expect(onConnected).toHaveBeenCalledTimes(1));
    expect(oauthApi.browserSignin).toHaveBeenCalledWith('garmin', COOKIES, {
      tosConsent: false,
      signal: expect.any(AbortSignal),
    });
  });

  it('connects nothing when the cookie read finishes after the window was closed', async () => {
    webView.handsOver = false;
    const onConnected = jest.fn();
    const { rerender } = render(
      <SciotteLoginModal visible onClose={jest.fn()} onConnected={onConnected} target="garmin" />,
    );

    fireEvent.press(screen.getByTestId('sciotte-browser-signin'));
    await waitFor(() => expect(webView.onSignedIn).not.toBeNull());
    const lateHandOver = webView.onSignedIn as (cookies: unknown[]) => void;

    rerender(<SciotteLoginModal visible={false} onClose={jest.fn()} onConnected={onConnected} target="garmin" />);
    lateHandOver(COOKIES);

    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(oauthApi.browserSignin).not.toHaveBeenCalled();
    expect(onConnected).not.toHaveBeenCalled();
  });
});

describe('readProviderCookies', () => {
  it('keeps the provider domain cookies of every host it passed through, once each', async () => {
    (CookieManager.get as jest.Mock).mockImplementation(async (url: string) =>
      url.includes('sso.garmin.com')
        ? { CASTGC: { name: 'CASTGC', value: 'tgc', domain: 'sso.garmin.com', path: '/', httpOnly: true, secure: true } }
        : {
            SESSIONID: { name: 'SESSIONID', value: 's', path: '/', httpOnly: true, secure: true },
            tracker: { name: 'tracker', value: 'x', domain: '.ads.example', path: '/' },
          },
    );

    const cookies = await readProviderCookies(['sso.garmin.com', 'connect.garmin.com'], 'garmin.com');

    expect(cookies.map((cookie) => `${cookie.name}@${cookie.domain}`).sort()).toEqual([
      'CASTGC@sso.garmin.com',
      'SESSIONID@connect.garmin.com',
    ]);
  });
});
