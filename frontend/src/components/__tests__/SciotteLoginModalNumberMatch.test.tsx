// ABOUTME: A number-match sign-in keeps polling while the number shows, so the athlete's phone tap is seen
// ABOUTME: Pins the re-poll after each poll that answers number_match again, and the connect it ends with

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import SciotteLoginModal from '../SciotteLoginModal';
import { ThemeProvider } from '../../hooks/useTheme';

const sciotteLogin = vi.fn();
const sciotteSelect2FA = vi.fn();

vi.mock('../../services/api', () => ({
  oauthApi: {
    sciotteLogin: (...args: unknown[]) => sciotteLogin(...args),
    sciotteSelect2FA: (...args: unknown[]) => sciotteSelect2FA(...args),
    sciotteSubmitOTP: vi.fn(),
    sciotteCancel: vi.fn().mockResolvedValue({ cancelled: false }),
    authorizeUrl: vi.fn(),
  },
}));
vi.mock('../OAuthAppSetupModal', () => ({ default: () => null }));

const NUMBER_MATCH = { status: 'number_match', number: '93' };

describe('SciotteLoginModal — a number-match sign-in', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('polls again while the number shows, until the tap connects', async () => {
    sciotteLogin.mockResolvedValue(NUMBER_MATCH);
    sciotteSelect2FA
      .mockResolvedValueOnce(NUMBER_MATCH)
      .mockResolvedValueOnce(NUMBER_MATCH)
      .mockResolvedValueOnce({ status: 'connected', provider: 'sciotte_garmin' });
    const onConnected = vi.fn();
    render(
      <ThemeProvider>
        <SciotteLoginModal isOpen onClose={vi.fn()} onConnected={onConnected} target="garmin" />
      </ThemeProvider>,
    );

    const user = userEvent.setup();
    await user.type(screen.getByLabelText(/email/i), 'athlete@example.com');
    await user.type(screen.getByLabelText('Password'), 'not-a-real-password');
    await user.click(screen.getByRole('button', { name: 'Log In' }));

    await vi.waitFor(() => expect(onConnected).toHaveBeenCalledTimes(1), { timeout: 8000 });
    expect(sciotteSelect2FA).toHaveBeenCalledTimes(3);
    for (const call of sciotteSelect2FA.mock.calls) expect(call[0]).toBe('poll');
  }, 15000);
});
