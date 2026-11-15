// ABOUTME: A Garmin sign-in hands the grant it still owes to the caller, which authorizes it right after (carnet#1494)
// ABOUTME: Pins the follow-on the server names reaching onConnected, and a sign-in that owes nothing passing none

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import SciotteLoginModal from '../SciotteLoginModal';
import { ThemeProvider } from '../../hooks/useTheme';

const sciotteLogin = vi.fn();

vi.mock('../../services/api', () => ({
  oauthApi: {
    sciotteLogin: (...args: unknown[]) => sciotteLogin(...args),
    sciotteSelect2FA: vi.fn(),
    sciotteSubmitOTP: vi.fn(),
    sciotteCancel: vi.fn().mockResolvedValue({ cancelled: false }),
    authorizeUrl: vi.fn(),
  },
}));
vi.mock('../OAuthAppSetupModal', () => ({ default: () => null }));

function renderModal(onConnected: (followOn?: string) => void) {
  return render(
    <ThemeProvider>
      <SciotteLoginModal isOpen onClose={vi.fn()} onConnected={onConnected} target="garmin" />
    </ThemeProvider>,
  );
}

async function signIn() {
  const user = userEvent.setup();
  await user.type(screen.getByLabelText(/email/i), 'athlete@example.com');
  await user.type(screen.getByLabelText('Password'), 'not-a-real-password');
  await user.click(screen.getByRole('button', { name: 'Log In' }));
}

describe('SciotteLoginModal — the grant a Garmin sign-in goes on to', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('hands the owed grant to the caller, which authorizes it right after', async () => {
    sciotteLogin.mockResolvedValue({
      status: 'connected',
      provider: 'sciotte_garmin',
      oauth_follow_on: { provider: 'garmin' },
    });
    const onConnected = vi.fn();
    renderModal(onConnected);

    await signIn();

    await vi.waitFor(() => expect(onConnected).toHaveBeenCalledWith('garmin'));
  });

  it('passes no follow-on when the sign-in owes nothing', async () => {
    sciotteLogin.mockResolvedValue({ status: 'connected', provider: 'sciotte_garmin', oauth_follow_on: null });
    const onConnected = vi.fn();
    renderModal(onConnected);

    await signIn();

    await vi.waitFor(() => expect(onConnected).toHaveBeenCalledTimes(1));
    expect(onConnected).toHaveBeenCalledWith(undefined);
  });
});
