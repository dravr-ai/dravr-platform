// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A connected Garmin sign-in ends the connect: onConnected takes no provider and no OAuth grant follows it
// ABOUTME: Red if the login, a 2FA pick or a code step hands a grant to authorize, even when the answer names one

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import SciotteLoginModal from '../SciotteLoginModal';
import { ThemeProvider } from '../../hooks/useTheme';

const sciotteLogin = vi.fn();
const sciotteSelect2FA = vi.fn();
const sciotteSubmitOTP = vi.fn();
const authorizeUrl = vi.fn();

vi.mock('../../services/api', () => ({
  oauthApi: {
    sciotteLogin: (...args: unknown[]) => sciotteLogin(...args),
    sciotteSelect2FA: (...args: unknown[]) => sciotteSelect2FA(...args),
    sciotteSubmitOTP: (...args: unknown[]) => sciotteSubmitOTP(...args),
    sciotteCancel: vi.fn().mockResolvedValue({ cancelled: false }),
    authorizeUrl: (...args: unknown[]) => authorizeUrl(...args),
  },
}));
vi.mock('../OAuthAppSetupModal', () => ({ default: () => null }));

/**
 * A connected answer that still names Garmin's grant: the modal must end the
 * connect all the same.
 */
const CONNECTED_NAMING_A_GRANT = {
  status: 'connected',
  provider: 'sciotte_garmin',
  oauth_follow_on: { provider: 'garmin', authorization_url: 'https://connect.garmin.test/oauth2Confirm' },
};

function renderModal() {
  const onConnected = vi.fn();
  render(
    <ThemeProvider>
      <SciotteLoginModal isOpen onClose={vi.fn()} onConnected={onConnected} target="garmin" />
    </ThemeProvider>,
  );
  return onConnected;
}

async function signIn(user: ReturnType<typeof userEvent.setup>) {
  await user.type(screen.getByLabelText(/email/i), 'athlete@example.com');
  await user.type(screen.getByLabelText('Password'), 'not-a-real-password');
  await user.click(screen.getByRole('button', { name: 'Log In' }));
}

/** onConnected ran once with no argument, and nothing started an authorization. */
async function endedConnected(onConnected: ReturnType<typeof vi.fn>) {
  await vi.waitFor(() => expect(onConnected).toHaveBeenCalledTimes(1));
  expect(onConnected.mock.calls).toEqual([[]]);
  expect(authorizeUrl).not.toHaveBeenCalled();
}

describe('SciotteLoginModal — a connected Garmin sign-in ends the connect', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('after the login', async () => {
    sciotteLogin.mockResolvedValue(CONNECTED_NAMING_A_GRANT);
    const onConnected = renderModal();

    await signIn(userEvent.setup());

    await endedConnected(onConnected);
  });

  it('after a 2FA pick', async () => {
    sciotteLogin.mockResolvedValue({ status: 'two_factor_choice', options: [{ id: 'sms', label: 'Text message' }] });
    sciotteSelect2FA.mockResolvedValue(CONNECTED_NAMING_A_GRANT);
    const onConnected = renderModal();
    const user = userEvent.setup();

    await signIn(user);
    await user.click(await screen.findByRole('button', { name: 'Text message' }));

    await endedConnected(onConnected);
  });

  it('after the code step', async () => {
    sciotteLogin.mockResolvedValue({ status: 'otp_required' });
    sciotteSubmitOTP.mockResolvedValue(CONNECTED_NAMING_A_GRANT);
    const onConnected = renderModal();
    const user = userEvent.setup();

    await signIn(user);
    await user.type(await screen.findByLabelText('Enter the 6-digit code Garmin sent you'), '246810');
    await user.click(screen.getByRole('button', { name: 'Verify' }));

    await endedConnected(onConnected);
  });
});
