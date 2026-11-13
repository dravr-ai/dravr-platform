// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the /sign-in-check page — the provider it names from its token, and what it tells the phone app that loaded it
// ABOUTME: Red if a connected takeover is not posted as connected, or one that ends without connecting is not posted as closed

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { act, render, screen, waitFor } from '@testing-library/react';
import { i18n } from '@pierre/i18n';
import SignInCheck from '../SignInCheck';
import { FakeTakeoverSocket, installFakeTakeoverSocket, lastSocket } from '../../test/fakeTakeoverSocket';

vi.mock('../../services/api', () => ({
  oauthApi: {
    sciotteTakeoverSocketUrl: (token: string) => `ws://app.dravr.test/ws/providers/sciotte/takeover?token=${token}`,
  },
}));

const b64url = (json: string) => btoa(json).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
const TOKEN = `${b64url('{"alg":"HS256"}')}.${b64url('{"tgt":"sciotte_garmin"}')}.sig`;

describe('SignInCheck', () => {
  const postMessage = vi.fn();

  beforeEach(() => {
    installFakeTakeoverSocket();
    postMessage.mockClear();
    (window as Window & { ReactNativeWebView?: unknown }).ReactNativeWebView = { postMessage };
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    delete (window as Window & { ReactNativeWebView?: unknown }).ReactNativeWebView;
  });

  async function running() {
    render(<SignInCheck token={TOKEN} />);
    await waitFor(() => expect(FakeTakeoverSocket.instances).toHaveLength(1));
    const socket = lastSocket();
    act(() => socket.ready());
    await screen.findByText(i18n.t('shell.sciotteTakeoverPrompt', { provider: 'Garmin' }));
    return socket;
  }

  it('names the provider its token is for, and tells the app when it connected', async () => {
    const socket = await running();
    act(() => socket.event({ type: 'outcome', status_code: 200, body: { status: 'connected', provider: 'sciotte_garmin' } }));
    expect(await screen.findByTestId('sign-in-check-connected')).toHaveTextContent(i18n.t('shell.sciotteConnected'));
    expect(postMessage).toHaveBeenCalledWith(JSON.stringify({ type: 'connected' }));
  });

  it('tells the app which grant a connected sign-in goes on to', async () => {
    const socket = await running();
    const body = { status: 'connected', provider: 'sciotte_garmin', oauth_follow_on: { provider: 'garmin' } };
    act(() => socket.event({ type: 'outcome', status_code: 200, body }));
    await screen.findByTestId('sign-in-check-connected');
    expect(postMessage).toHaveBeenCalledWith(JSON.stringify({ type: 'connected', follow_on: 'garmin' }));
  });

  it('opens the grant authorization a hosted sign-in goes on to, with no app to tell', async () => {
    delete (window as Window & { ReactNativeWebView?: unknown }).ReactNativeWebView;
    const assign = vi.fn();
    vi.stubGlobal('location', { ...window.location, assign });
    const socket = await running();
    const authorization_url = 'https://connect.garmin.test/oauth2Confirm?state=s';
    const body = {
      status: 'connected',
      provider: 'sciotte_garmin',
      oauth_follow_on: { provider: 'garmin', authorization_url },
    };
    act(() => socket.event({ type: 'outcome', status_code: 200, body }));
    await screen.findByTestId('sign-in-check-connected');
    expect(assign).toHaveBeenCalledWith(authorization_url);
    expect(postMessage).not.toHaveBeenCalled();
  });

  it('tells the app the takeover closed, with the answer it ended with', async () => {
    const socket = await running();
    const outcome = { type: 'outcome', status_code: 200, body: { status: 'failed', error: 'x' } };
    act(() => socket.event(outcome));
    expect(await screen.findByTestId('sign-in-check-closed')).toHaveTextContent(i18n.t('auth.loginFailed'));
    expect(postMessage).toHaveBeenCalledWith(JSON.stringify({ type: 'closed', outcome }));
  });

  it('tells the app the takeover closed when its socket drops', async () => {
    const socket = await running();
    act(() => socket.serverClose());
    expect(await screen.findByTestId('sign-in-check-closed')).toHaveTextContent(
      i18n.t('shell.sciotteTakeoverEnded', { provider: 'Garmin' }),
    );
    expect(postMessage).toHaveBeenCalledWith(JSON.stringify({ type: 'closed' }));
  });

  it('opens no socket for a link that carried no token', () => {
    render(<SignInCheck token={null} />);
    expect(screen.getByTestId('sign-in-check-closed')).toBeInTheDocument();
    expect(FakeTakeoverSocket.instances).toHaveLength(0);
  });
});
