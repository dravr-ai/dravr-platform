// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the hosted sign-in's return path as a route: its deep link steps back to the login it covered
// ABOUTME: Without the route Expo Router renders "Unmatched Route" when the redirect also arrives as a deep link

import React from 'react';
import { render } from '@testing-library/react-native';

const mockRouter = { back: jest.fn(), replace: jest.fn(), canGoBack: jest.fn(() => true) };

jest.mock('expo-router', () => ({
  useRouter: () => mockRouter,
}));

import SignInCallbackRoute from '../app/(auth)/auth/callback';

describe('the sign-in callback route', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('renders nothing and steps back to the login screen under it', () => {
    mockRouter.canGoBack.mockReturnValue(true);

    const view = render(<SignInCallbackRoute />);

    expect(view.toJSON()).toBeNull();
    expect(mockRouter.back).toHaveBeenCalledTimes(1);
    expect(mockRouter.replace).not.toHaveBeenCalled();
  });

  it('opens the login screen when the link started the app', () => {
    mockRouter.canGoBack.mockReturnValue(false);

    render(<SignInCallbackRoute />);

    expect(mockRouter.replace).toHaveBeenCalledWith('/(auth)/login');
    expect(mockRouter.back).not.toHaveBeenCalled();
  });
});
