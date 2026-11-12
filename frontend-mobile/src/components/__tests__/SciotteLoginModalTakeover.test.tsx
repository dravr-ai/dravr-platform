// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A provider robot check opens the web app's takeover page in the mobile sign-in sheet, which reads its connected and closed (ADR-034)
// ABOUTME: Red if the WebView loads anything but the takeover link, a posted answer skips the login handling, or a close leaks the attempt

import React from 'react';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react-native';
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
    sciotteCancel: jest.fn(),
    initMobileOAuth: jest.fn(),
  },
}));
jest.mock('../OAuthAppSetupModal', () => ({ OAuthAppSetupModal: () => null }));

/** What the sheet handed the WebView: the page it loads, the navigations it allows, its message handler. */
interface WebViewProps {
  source: { uri: string };
  onMessage: (event: { nativeEvent: { data: string } }) => void;
  onShouldStartLoadWithRequest: (request: { url: string }) => boolean;
  testID?: string;
}
let mockWebView: WebViewProps | null = null;
jest.mock('react-native-webview', () => {
  const ReactActual = jest.requireActual<typeof import('react')>('react');
  const { View } = jest.requireActual<typeof import('react-native')>('react-native');
  return {
    WebView: (props: WebViewProps) => {
      mockWebView = props;
      return ReactActual.createElement(View, { testID: props.testID });
    },
  };
});

const sciotteLogin = oauthApi.sciotteLogin as jest.Mock;
const sciotteCancel = oauthApi.sciotteCancel as jest.Mock;

const TAKEOVER_URL = 'https://app.dravr.test/sign-in-check#token=abc.def.ghi';

interface Handlers {
  onClose: jest.Mock;
  onConnected: jest.Mock;
  onFallBackToOAuth?: jest.Mock;
}

function sheet(visible: boolean, handlers: Handlers) {
  return <SciotteLoginModal visible={visible} target="garmin" {...handlers} />;
}

/** Sign in, and have the provider stop the login at a robot check. */
async function intoTakeover(extra: Partial<Handlers> = {}) {
  const handlers: Handlers = { onClose: jest.fn(), onConnected: jest.fn(), ...extra };
  const rendered = render(sheet(true, handlers));
  fireEvent.changeText(screen.getByTestId('sciotte-email'), 'athlete@example.test');
  fireEvent.changeText(screen.getByTestId('sciotte-password'), 'not-a-real-password');
  fireEvent.press(screen.getByTestId('sciotte-login-submit'));
  await screen.findByTestId('sciotte-takeover-webview');
  const attemptId = (sciotteLogin.mock.calls[0][1] as { attemptId: string }).attemptId;
  return { ...rendered, handlers, attemptId };
}

/** The takeover page posts `message`. */
function post(message: Record<string, unknown>): void {
  act(() => {
    mockWebView?.onMessage({ nativeEvent: { data: JSON.stringify(message) } });
  });
}

describe('SciotteLoginModal (mobile) — the takeover of a provider robot check', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockWebView = null;
    sciotteLogin.mockResolvedValue({ status: 'takeover_required', provider: 'sciotte_garmin', takeover_url: TAKEOVER_URL });
    sciotteCancel.mockResolvedValue({ cancelled: true });
  });

  it("loads the takeover link, and never navigates off the web app's origin", async () => {
    await intoTakeover();
    expect(mockWebView?.source).toEqual({ uri: TAKEOVER_URL });
    expect(mockWebView?.onShouldStartLoadWithRequest({ url: TAKEOVER_URL })).toBe(true);
    expect(mockWebView?.onShouldStartLoadWithRequest({ url: 'https://sso.garmin.com/signin' })).toBe(false);
  });

  it('ends on the success the login would show when the page says connected', async () => {
    const { handlers } = await intoTakeover();
    post({ type: 'connected' });
    expect(await screen.findByTestId('sciotte-success-line')).toBeTruthy();
    expect(handlers.onConnected).toHaveBeenCalledTimes(1);
    expect(sciotteCancel).not.toHaveBeenCalled();
  });

  it('reads the answer a closed takeover carries as the login answer', async () => {
    await intoTakeover();
    post({ type: 'closed', outcome: { type: 'outcome', status_code: 200, body: { status: 'otp_required' } } });
    expect(await screen.findByTestId('sciotte-otp')).toBeTruthy();
  });

  it('says the sign-in window closed when the page closed with nothing to say', async () => {
    await intoTakeover();
    post({ type: 'closed' });
    expect(await screen.findByText(i18n.t('shell.sciotteTakeoverEnded', { provider: 'Garmin' }))).toBeTruthy();
  });

  it('tries again under a new attempt, since the server cancelled the one the takeover held', async () => {
    const { attemptId } = await intoTakeover();
    post({ type: 'closed' });
    fireEvent.press(await screen.findByText(i18n.t('app.tryAgain')));
    sciotteLogin.mockResolvedValue({ status: 'connected', provider: 'sciotte_garmin' });
    fireEvent.changeText(screen.getByTestId('sciotte-email'), 'athlete@example.test');
    fireEvent.changeText(screen.getByTestId('sciotte-password'), 'not-a-real-password');
    fireEvent.press(screen.getByTestId('sciotte-login-submit'));
    await waitFor(() => expect(sciotteLogin).toHaveBeenCalledTimes(2));
    const retried = (sciotteLogin.mock.calls[1][1] as { attemptId: string }).attemptId;
    expect(retried).toBeTruthy();
    expect(retried).not.toBe(attemptId);
  });

  it('closing mid-takeover cancels the attempt the takeover holds', async () => {
    const { handlers, rerender, attemptId } = await intoTakeover();
    fireEvent.press(screen.getByTestId('sciotte-modal-close'));
    expect(handlers.onClose).toHaveBeenCalledTimes(1);

    rerender(sheet(false, handlers));
    await waitFor(() => expect(sciotteCancel).toHaveBeenCalledWith(attemptId));
  });

  it("hands a close without finishing to the card's OAuth when it offers one", async () => {
    const onFallBackToOAuth = jest.fn();
    const { handlers } = await intoTakeover({ onFallBackToOAuth });
    fireEvent.press(screen.getByTestId('sciotte-modal-close'));
    expect(onFallBackToOAuth).toHaveBeenCalledTimes(1);
    expect(handlers.onClose).not.toHaveBeenCalled();
  });

  it('a page that closed is an ended sign-in, never the OAuth fallback', async () => {
    const onFallBackToOAuth = jest.fn();
    await intoTakeover({ onFallBackToOAuth });
    post({ type: 'closed' });
    expect(await screen.findByText(i18n.t('shell.sciotteTakeoverEnded', { provider: 'Garmin' }))).toBeTruthy();
    expect(onFallBackToOAuth).not.toHaveBeenCalled();
  });
});
