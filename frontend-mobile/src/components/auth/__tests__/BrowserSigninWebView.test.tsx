// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: carnet#1617 — the phone's sign-in WebView hands the provider's cookies over once it lands, then clears them from the phone
// ABOUTME: Drives the WebView's navigations through a mock that keeps its onNavigationStateChange prop

import React from 'react';
import { render, waitFor } from '@testing-library/react-native';
import CookieManager from '@react-native-cookies/cookies';
import { BrowserSigninWebView } from '../BrowserSigninWebView';

let navigate: ((navigation: { url: string }) => void) | undefined;
jest.mock('react-native-webview', () => {
  const ReactActual = jest.requireActual('react');
  const { View } = jest.requireActual('react-native');
  return {
    WebView: (props: { onNavigationStateChange: (navigation: { url: string }) => void; testID?: string }) => {
      navigate = props.onNavigationStateChange;
      return ReactActual.createElement(View, { testID: props.testID });
    },
  };
});

const GARMIN = {
  provider: 'garmin',
  login_url: 'https://sso.garmin.com/portal/sso/en-US/sign-in',
  success_patterns: ['/app'],
  cookie_domain: 'garmin.com',
};

describe('BrowserSigninWebView', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    (CookieManager.get as jest.Mock).mockResolvedValue({
      SESSIONID: { name: 'SESSIONID', value: 's', domain: 'connect.garmin.com', path: '/', httpOnly: true, secure: true },
    });
  });

  it('hands the cookies over once it lands signed in, and clears them from the phone', async () => {
    const onSignedIn = jest.fn();
    render(<BrowserSigninWebView entry={GARMIN} onSignedIn={onSignedIn} onError={jest.fn()} height={400} />);

    navigate?.({ url: 'https://sso.garmin.com/portal/sso/en-US/sign-in' });
    expect(onSignedIn).not.toHaveBeenCalled();
    navigate?.({ url: 'https://connect.garmin.com/app' });

    await waitFor(() => expect(onSignedIn).toHaveBeenCalledTimes(1));
    expect(onSignedIn.mock.calls[0][0].map((cookie: { name: string }) => cookie.name)).toEqual(['SESSIONID']);
    expect(CookieManager.clearAll).toHaveBeenCalledWith(true);
  });

  it('hands over once, however many signed-in pages follow', async () => {
    const onSignedIn = jest.fn();
    render(<BrowserSigninWebView entry={GARMIN} onSignedIn={onSignedIn} onError={jest.fn()} height={400} />);

    navigate?.({ url: 'https://connect.garmin.com/app' });
    navigate?.({ url: 'https://connect.garmin.com/app/home' });

    await waitFor(() => expect(onSignedIn).toHaveBeenCalledTimes(1));
    expect(CookieManager.get).toHaveBeenCalled();
  });
});
