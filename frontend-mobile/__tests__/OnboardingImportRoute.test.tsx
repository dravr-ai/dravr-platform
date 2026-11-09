// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The onboarding import route opens straight on the source a card's export link or the history sheet names
// ABOUTME: Without a source it opens on the list of sources, and its close steps back to the connect step

import React from 'react';
import { fireEvent, render, screen } from '@testing-library/react-native';

const mockRouter = { back: jest.fn(), push: jest.fn(), replace: jest.fn(), canGoBack: () => true };
let mockParams: { source?: string } = {};

jest.mock('expo-router', () =>
  require('../jest.expo-router').createExpoRouterMock({
    useRouter: () => mockRouter,
    useLocalSearchParams: () => mockParams,
  }),
);
// The screen, reduced to what the route hands it.
jest.mock('../src/screens/import/ImportScreen', () => {
  const React = require('react');
  const { Text } = require('react-native');
  return {
    ImportScreen: ({ initialSource, onClose }: { initialSource?: string; onClose?: () => void }) =>
      React.createElement(Text, { onPress: onClose, testID: 'import-screen' }, `import:${initialSource ?? 'list'}`),
  };
});

import OnboardingImportRoute from '../app/(onboarding)/import';

describe('the onboarding import route', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockParams = {};
  });

  it('opens on the Garmin export a link named', () => {
    mockParams = { source: 'garmin_archive' };

    render(<OnboardingImportRoute />);

    expect(screen.getByTestId('import-screen')).toHaveTextContent('import:garmin_archive');
  });

  it('opens on the list of sources when no source is named, and closes back to the connect step', () => {
    render(<OnboardingImportRoute />);

    expect(screen.getByTestId('import-screen')).toHaveTextContent('import:list');
    fireEvent.press(screen.getByTestId('import-screen'));
    expect(mockRouter.back).toHaveBeenCalledTimes(1);
  });
});
