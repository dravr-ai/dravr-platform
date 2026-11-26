// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A number-match sign-in keeps polling while the number shows, so the athlete's phone tap is seen
// ABOUTME: Pins the re-poll after each poll that answers number_match again, and the connect it ends with,
// ABOUTME: and leaving the number for another method only once the poll in flight has answered

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
  },
}));
jest.mock('../OAuthAppSetupModal', () => ({ OAuthAppSetupModal: () => null }));

const sciotteLogin = oauthApi.sciotteLogin as jest.Mock;
const sciotteSelect2FA = oauthApi.sciotteSelect2FA as jest.Mock;
const NUMBER_MATCH = { status: 'number_match', number: '93' };

describe('SciotteLoginModal (mobile) — a number-match sign-in', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('polls again while the number shows, until the tap connects', async () => {
    sciotteLogin.mockResolvedValue(NUMBER_MATCH);
    sciotteSelect2FA
      .mockResolvedValueOnce(NUMBER_MATCH)
      .mockResolvedValueOnce(NUMBER_MATCH)
      .mockResolvedValueOnce({ status: 'connected', provider: 'sciotte_garmin' });
    const onConnected = jest.fn();

    render(<SciotteLoginModal visible onClose={jest.fn()} onConnected={onConnected} target="garmin" />);
    fireEvent.changeText(screen.getByTestId('sciotte-email'), 'athlete@example.test');
    fireEvent.changeText(screen.getByTestId('sciotte-password'), 'not-a-real-password');
    fireEvent.press(screen.getByTestId('sciotte-login-submit'));

    await waitFor(() => expect(onConnected).toHaveBeenCalledTimes(1), { timeout: 8000 });
    expect(sciotteSelect2FA).toHaveBeenCalledTimes(3);
    for (const call of sciotteSelect2FA.mock.calls) expect(call[0]).toBe('poll');
  }, 15000);

  it('leaves the number for another method once the poll in flight answers', async () => {
    sciotteLogin.mockResolvedValue(NUMBER_MATCH);
    let answerPoll: (value: unknown) => void = () => undefined;
    sciotteSelect2FA
      .mockImplementationOnce(() => new Promise((resolve) => { answerPoll = resolve; }))
      .mockResolvedValueOnce({
        status: 'two_factor_choice',
        options: [{ id: 'otp', label: 'Get a code from Google Authenticator' }],
      });

    render(<SciotteLoginModal visible onClose={jest.fn()} onConnected={jest.fn()} target="garmin" />);
    fireEvent.changeText(screen.getByTestId('sciotte-email'), 'athlete@example.test');
    fireEvent.changeText(screen.getByTestId('sciotte-password'), 'not-a-real-password');
    fireEvent.press(screen.getByTestId('sciotte-login-submit'));

    // The first poll is in flight: asking for another method sends nothing yet,
    // since the poll holds the flow until it answers.
    await waitFor(() => expect(sciotteSelect2FA).toHaveBeenCalledTimes(1));
    fireEvent.press(await screen.findByTestId('sciotte-another-way'));
    expect(sciotteSelect2FA).toHaveBeenCalledTimes(1);

    answerPoll(NUMBER_MATCH);
    expect(await screen.findByText('Get a code from Google Authenticator')).toBeTruthy();
    expect(sciotteSelect2FA.mock.calls.map((call) => call[0])).toEqual(['poll', 'another_way']);
  }, 15000);

  it('forgets another method a poll overtook, so a retry keeps polling its new number', async () => {
    sciotteLogin.mockResolvedValue(NUMBER_MATCH);
    let failPoll: (reason: unknown) => void = () => undefined;
    sciotteSelect2FA
      .mockImplementationOnce(() => new Promise((_, reject) => { failPoll = reject; }))
      .mockResolvedValueOnce({ status: 'connected', provider: 'sciotte_garmin' });
    const onConnected = jest.fn();

    render(<SciotteLoginModal visible onClose={jest.fn()} onConnected={onConnected} target="garmin" />);
    fireEvent.changeText(screen.getByTestId('sciotte-email'), 'athlete@example.test');
    fireEvent.changeText(screen.getByTestId('sciotte-password'), 'not-a-real-password');
    fireEvent.press(screen.getByTestId('sciotte-login-submit'));
    await waitFor(() => expect(sciotteSelect2FA).toHaveBeenCalledTimes(1));

    // Asked for while the poll runs, then overtaken: the poll ends the sign-in.
    fireEvent.press(await screen.findByTestId('sciotte-another-way'));
    failPoll(new Error('the sign-in ended'));
    fireEvent.press(await screen.findByText('Try Again'));

    // The retry's number is polled, never left for another method unasked.
    fireEvent.press(await screen.findByTestId('sciotte-login-submit'));
    await waitFor(() => expect(onConnected).toHaveBeenCalledTimes(1), { timeout: 8000 });
    expect(sciotteSelect2FA.mock.calls.map((call) => call[0])).toEqual(['poll', 'poll']);
  }, 15000);
});

