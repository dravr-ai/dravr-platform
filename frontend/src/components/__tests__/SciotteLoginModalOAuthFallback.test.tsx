// ABOUTME: A sign-in that cannot complete offers the provider's OAuth, started by the athlete's click (carnet#1477, carnet#1667)
// ABOUTME: Pins that every failure offers it beside the retry, on Garmin and Strava cards, and a window offered no fallback

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import SciotteLoginModal from '../SciotteLoginModal';
import { ThemeProvider } from '../../hooks/useTheme';
import type { SciotteTarget } from '@pierre/shared-types';

const sciotteLogin = vi.fn();
const sciotteSubmitOTP = vi.fn();

vi.mock('../../services/api', () => ({
  oauthApi: {
    sciotteLogin: (...args: unknown[]) => sciotteLogin(...args),
    sciotteSelect2FA: vi.fn(),
    sciotteSubmitOTP: (...args: unknown[]) => sciotteSubmitOTP(...args),
    sciotteCancel: vi.fn().mockResolvedValue({ cancelled: false }),
    authorizeUrl: vi.fn(),
  },
}));
vi.mock('../OAuthAppSetupModal', () => ({ default: () => null }));

function renderModal(onFallBackToOAuth?: () => void, target: SciotteTarget = 'garmin') {
  return render(
    <ThemeProvider>
      <SciotteLoginModal
        isOpen
        onClose={vi.fn()}
        onConnected={vi.fn()}
        target={target}
        onFallBackToOAuth={onFallBackToOAuth}
      />
    </ThemeProvider>,
  );
}

async function signIn() {
  const user = userEvent.setup();
  await user.type(screen.getByLabelText(/email/i), 'athlete@example.com');
  await user.type(screen.getByLabelText('Password'), 'not-a-real-password');
  await user.click(screen.getByRole('button', { name: 'Log In' }));
}

const refusal = (reason: string) => ({
  response: { status: 400, data: { code: 'InvalidInput', message: 'x', details: { reason } } },
});

describe('SciotteLoginModal — the OAuth a failed sign-in falls back to', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  for (const reason of ['sign_in_unavailable', 'login_challenge']) {
    it(`offers the provider's OAuth on a ${reason} refusal, started by the athlete's click`, async () => {
      sciotteLogin.mockRejectedValue(refusal(reason));
      const onFallBackToOAuth = vi.fn();
      renderModal(onFallBackToOAuth);

      await signIn();

      // A click, never on its own: the OAuth opens in a new tab, which a
      // browser only lets a click open.
      const offer = await screen.findByTestId('sciotte-oauth-fallback');
      expect(offer).toHaveTextContent('Continue with Garmin');
      expect(onFallBackToOAuth).not.toHaveBeenCalled();
      await userEvent.setup().click(offer);
      expect(onFallBackToOAuth).toHaveBeenCalledTimes(1);
    });
  }

  // An account the provider does not know by that email (one opened with
  // Google or Apple has no password there) answers like wrong credentials:
  // the athlete may retype, or connect the same account through OAuth.
  it('offers both a retry and the OAuth when the provider turns the credentials down', async () => {
    sciotteLogin.mockResolvedValue({ status: 'failed', error: "Couldn't find this account" });
    const onFallBackToOAuth = vi.fn();
    renderModal(onFallBackToOAuth);

    await signIn();

    expect(await screen.findByText(/Couldn't find this account/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /try again|retry/i })).toBeInTheDocument();
    const offer = screen.getByTestId('sciotte-oauth-fallback');
    expect(offer).toHaveTextContent('Continue with Garmin');
    expect(onFallBackToOAuth).not.toHaveBeenCalled();
    await userEvent.setup().click(offer);
    expect(onFallBackToOAuth).toHaveBeenCalledTimes(1);
  });

  it('keeps a turned-down sign-in to a retry when its card has no OAuth', async () => {
    sciotteLogin.mockResolvedValue({ status: 'failed', error: 'Invalid email or password' });
    renderModal(undefined);

    await signIn();

    expect(await screen.findByRole('button', { name: /try again|retry/i })).toBeInTheDocument();
    expect(screen.queryByTestId('sciotte-oauth-fallback')).not.toBeInTheDocument();
  });

  // A sign-in window never dead-ends: any other refusal keeps the athlete in
  // the window with its retry, and the card's OAuth beside it.
  it('keeps any other refusal in the window, offering the OAuth beside the retry', async () => {
    sciotteLogin.mockRejectedValue(refusal('login_flow_expired'));
    const onFallBackToOAuth = vi.fn();
    renderModal(onFallBackToOAuth);

    await signIn();

    expect(await screen.findByRole('button', { name: /try again|retry|start again/i })).toBeInTheDocument();
    const offer = screen.getByTestId('sciotte-oauth-fallback');
    expect(offer).toHaveTextContent('Continue with Garmin');
    expect(onFallBackToOAuth).not.toHaveBeenCalled();
    await userEvent.setup().click(offer);
    expect(onFallBackToOAuth).toHaveBeenCalledTimes(1);
  });

  it('offers the OAuth beside the retry when the code step fails', async () => {
    sciotteLogin.mockResolvedValue({ status: 'otp_required' });
    sciotteSubmitOTP.mockRejectedValue(refusal('login_flow_expired'));
    const onFallBackToOAuth = vi.fn();
    renderModal(onFallBackToOAuth);

    await signIn();
    const user = userEvent.setup();
    await user.type(await screen.findByPlaceholderText(/code/i), '123456');
    await user.click(screen.getByRole('button', { name: /verify/i }));

    expect(await screen.findByRole('button', { name: /try again/i })).toBeInTheDocument();
    expect(screen.getByTestId('sciotte-oauth-fallback')).toHaveTextContent('Continue with Garmin');
    expect(onFallBackToOAuth).not.toHaveBeenCalled();
  });

  // The 2026-11-24 dead end: Strava turned an email sign-in down for an
  // account opened with Google or Apple. The window quotes Strava's reason,
  // points at those methods, and offers Strava's OAuth beside the retry.
  it('on a Strava card, quotes a turned-down email sign-in, advises Google or Apple and offers the OAuth', async () => {
    sciotteLogin.mockResolvedValue({ status: 'failed', error: "Couldn't find this account" });
    const onFallBackToOAuth = vi.fn();
    renderModal(onFallBackToOAuth, 'strava');

    await userEvent.setup().click(screen.getByRole('button', { name: 'Log in with Strava email' }));
    await signIn();

    const message = await screen.findByText(/Strava turned the sign-in down: “Couldn't find this account”/);
    expect(message).toHaveTextContent('If you open Strava with Google or Apple, try again and pick that option.');
    expect(screen.getByRole('button', { name: /try again/i })).toBeInTheDocument();
    const offer = screen.getByTestId('sciotte-oauth-fallback');
    expect(offer).toHaveTextContent('Continue with Strava');
    await userEvent.setup().click(offer);
    expect(onFallBackToOAuth).toHaveBeenCalledTimes(1);
  });

  it('shows the failure when its card offers no OAuth to fall back to', async () => {
    sciotteLogin.mockRejectedValue(refusal('sign_in_unavailable'));
    renderModal(undefined);

    await signIn();

    expect(await screen.findByRole('button', { name: /try again|retry/i })).toBeInTheDocument();
    expect(screen.queryByTestId('sciotte-oauth-fallback')).not.toBeInTheDocument();
  });
});
