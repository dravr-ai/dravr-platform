// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Asserts a modal's Close button dismisses the screen, and lands on Home when nothing is beneath it
// ABOUTME: Home is the athlete's landing, so a modal opened from a deep link closes onto it rather than the chat list

import React from 'react';
import { fireEvent, render, screen } from '@testing-library/react-native';
import { HOME_ROUTE } from '../src/navigation/routes';
import { HeaderCloseButton } from '../src/components/ui/HeaderCloseButton';

let mockCanGoBack = true;
const mockRouter = {
  push: jest.fn(),
  replace: jest.fn(),
  back: jest.fn(),
  navigate: jest.fn(),
  canGoBack: () => mockCanGoBack,
};

jest.mock('expo-router', () =>
  require('../jest.expo-router').createExpoRouterMock({ useRouter: () => mockRouter }),
);

describe('HeaderCloseButton', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('goes back when a screen is beneath the modal', () => {
    mockCanGoBack = true;
    render(<HeaderCloseButton />);
    fireEvent.press(screen.getByTestId('back-button'));
    expect(mockRouter.back).toHaveBeenCalledTimes(1);
    expect(mockRouter.replace).not.toHaveBeenCalled();
  });

  it('lands on Home when nothing is beneath the modal', () => {
    mockCanGoBack = false;
    render(<HeaderCloseButton />);
    fireEvent.press(screen.getByTestId('back-button'));
    expect(mockRouter.replace).toHaveBeenCalledWith(HOME_ROUTE);
    expect(mockRouter.back).not.toHaveBeenCalled();
  });
});
