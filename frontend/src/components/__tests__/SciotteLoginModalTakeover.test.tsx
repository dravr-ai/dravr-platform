// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A provider robot check opens its takeover inside the sign-in window, and the takeover's answer ends it as the login's would (ADR-034)
// ABOUTME: Red if the token is not the link's, an outcome skips the login handling, a close leaks the attempt, or the OAuth fallback fires early

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { i18n } from '@pierre/i18n';
import SciotteLoginModal from '../SciotteLoginModal';
import { ThemeProvider } from '../../hooks/useTheme';
import { FakeTakeoverSocket, installFakeTakeoverSocket, lastSocket } from '../../test/fakeTakeoverSocket';

const sciotteLogin = vi.fn();
const sciotteCancel = vi.fn();

vi.mock('../../services/api', () => ({
  oauthApi: {
    sciotteLogin: (...args: unknown[]) => sciotteLogin(...args),
    sciotteSelect2FA: vi.fn(),
    sciotteSubmitOTP: vi.fn(),
    sciotteCancel: (...args: unknown[]) => sciotteCancel(...args),
    authorizeUrl: vi.fn(),
    sciotteTakeoverSocketUrl: (token: string) => `ws://app.dravr.test/ws/providers/sciotte/takeover?token=${token}`,
  },
}));
vi.mock('../OAuthAppSetupModal', () => ({ default: () => null }));

const TOKEN = 'eyJ0.eyJ0Z3QiOiJzY2lvdHRlX2dhcm1pbiJ9.sig';
const TAKEOVER = {
  status: 'takeover_required',
  provider: 'sciotte_garmin',
  takeover_url: `https://app.dravr.test/sign-in-check#token=${encodeURIComponent(TOKEN)}`,
};
const PROMPT = () => i18n.t('shell.sciotteTakeoverPrompt', { provider: 'Garmin' });

interface Handlers {
  onClose: () => void;
  onConnected: () => void;
  onFallBackToOAuth?: () => void;
}

function modal(isOpen: boolean, handlers: Handlers) {
  return (
    <ThemeProvider>
      <SciotteLoginModal isOpen={isOpen} target="garmin" {...handlers} />
    </ThemeProvider>
  );
}

/** Sign in, and have the provider stop the login at a robot check whose page streams. */
async function intoTakeover(handlers: Partial<Handlers> = {}) {
  const all: Handlers = { onClose: vi.fn(), onConnected: vi.fn(), ...handlers };
  const rendered = render(modal(true, all));
  const user = userEvent.setup();
  await user.type(screen.getByLabelText(/email/i), 'athlete@example.com');
  await user.type(screen.getByLabelText('Password'), 'not-a-real-password');
  await user.click(screen.getByRole('button', { name: 'Log In' }));
  await waitFor(() => expect(FakeTakeoverSocket.instances).toHaveLength(1));
  const socket = lastSocket();
  act(() => socket.ready());
  await screen.findByText(PROMPT());
  const attemptId = (sciotteLogin.mock.calls[0][1] as { attemptId: string }).attemptId;
  return { ...rendered, handlers: all, socket, user, attemptId };
}

describe('SciotteLoginModal — the takeover of a provider robot check', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    installFakeTakeoverSocket();
    sciotteLogin.mockResolvedValue(TAKEOVER);
    sciotteCancel.mockResolvedValue({ cancelled: true });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("opens the takeover in the window with the token from the link's fragment", async () => {
    const { socket } = await intoTakeover();
    expect(socket.url).toBe(`ws://app.dravr.test/ws/providers/sciotte/takeover?token=${TOKEN}`);
    expect(screen.getByTestId('takeover-viewer')).toBeInTheDocument();
    expect(screen.queryByTestId('takeover-oauth-offer')).not.toBeInTheDocument();
  });

  it('ends on the success the login would show when the takeover connects', async () => {
    const { socket, handlers } = await intoTakeover();
    act(() => socket.event({ type: 'outcome', status_code: 200, body: { status: 'connected', provider: 'sciotte_garmin' } }));
    // The header's status line and the success body both say so.
    expect((await screen.findAllByText(i18n.t('shell.sciotteConnected'))).length).toBeGreaterThan(0);
    expect(handlers.onConnected).toHaveBeenCalledTimes(1);
    expect(sciotteCancel).not.toHaveBeenCalled();
  });

  it('shows wrong credentials as the login does, for another try', async () => {
    const { socket } = await intoTakeover();
    act(() => socket.event({ type: 'outcome', status_code: 200, body: { status: 'failed', error: 'Invalid email or password' } }));
    expect(await screen.findByText(i18n.t('shell.sciotteLoginFailed'))).toBeInTheDocument();
    expect(screen.getByRole('button', { name: i18n.t('chat.tryAgain') })).toBeInTheDocument();
  });

  it('continues to the code step when the provider asks for one after the check', async () => {
    const { socket } = await intoTakeover();
    act(() => socket.event({ type: 'outcome', status_code: 200, body: { status: 'otp_required' } }));
    expect(await screen.findByLabelText(/code/i)).toBeInTheDocument();
  });

  it('says the sign-in window closed when the takeover ends with nothing to say', async () => {
    const { socket } = await intoTakeover();
    act(() => socket.event({ type: 'outcome', status_code: 400, body: { details: { reason: 'takeover_ended' } } }));
    expect(await screen.findByText(i18n.t('shell.sciotteTakeoverEnded', { provider: 'Garmin' }))).toBeInTheDocument();
  });

  it('says the same when the socket drops', async () => {
    const { socket } = await intoTakeover();
    act(() => socket.serverClose());
    expect(await screen.findByText(i18n.t('shell.sciotteTakeoverEnded', { provider: 'Garmin' }))).toBeInTheDocument();
  });

  it('tries again under a new attempt, since the server cancelled the one the takeover held', async () => {
    const { socket, user, attemptId } = await intoTakeover();
    act(() => socket.serverClose());
    await user.click(await screen.findByRole('button', { name: i18n.t('chat.tryAgain') }));
    sciotteLogin.mockResolvedValue({ status: 'connected', provider: 'sciotte_garmin' });
    await user.type(screen.getByLabelText(/email/i), 'athlete@example.com');
    await user.type(screen.getByLabelText('Password'), 'not-a-real-password');
    await user.click(screen.getByRole('button', { name: 'Log In' }));
    await waitFor(() => expect(sciotteLogin).toHaveBeenCalledTimes(2));
    const retried = (sciotteLogin.mock.calls[1][1] as { attemptId: string }).attemptId;
    expect(retried).toMatch(/^[0-9a-f-]{36}$/);
    expect(retried).not.toBe(attemptId);
  });

  it('hands a refusal the card falls back on to its OAuth, as the login does', async () => {
    const onFallBackToOAuth = vi.fn();
    const { socket } = await intoTakeover({ onFallBackToOAuth });
    act(() =>
      socket.event({ type: 'outcome', status_code: 503, body: { code: 'x', details: { reason: 'sign_in_unavailable' } } }),
    );
    await waitFor(() => expect(onFallBackToOAuth).toHaveBeenCalledTimes(1));
  });

  it('closing mid-takeover cancels the attempt and closes the socket, with no fallback to offer', async () => {
    const { socket, handlers, user, rerender, attemptId } = await intoTakeover();
    await user.click(screen.getByRole('button', { name: i18n.t('shell.sciotteCloseModalAria') }));
    expect(handlers.onClose).toHaveBeenCalledTimes(1);

    rerender(modal(false, handlers));
    expect(sciotteCancel).toHaveBeenCalledWith(attemptId);
    expect(socket.closes).toEqual([1000]);
  });

  it("offers the card's OAuth in the view, and hands a close without finishing to it", async () => {
    const onFallBackToOAuth = vi.fn();
    const { handlers, user } = await intoTakeover({ onFallBackToOAuth });
    expect(screen.getByTestId('takeover-oauth-offer')).toHaveTextContent(
      i18n.t('shell.sciotteTakeoverOAuthOffer', { provider: 'Garmin' }),
    );

    await user.click(screen.getByRole('button', { name: i18n.t('shell.sciotteCloseModalAria') }));

    expect(onFallBackToOAuth).toHaveBeenCalledTimes(1);
    expect(handlers.onClose).not.toHaveBeenCalled();
  });

  it('closes, without the fallback, before the takeover begins', async () => {
    sciotteLogin.mockReset();
    const onFallBackToOAuth = vi.fn();
    const onClose = vi.fn();
    render(modal(true, { onClose, onConnected: vi.fn(), onFallBackToOAuth }));
    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: i18n.t('shell.sciotteCloseModalAria') }));
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(onFallBackToOAuth).not.toHaveBeenCalled();
  });
});
