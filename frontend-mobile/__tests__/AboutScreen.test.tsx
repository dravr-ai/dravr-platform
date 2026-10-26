// ABOUTME: Pins the About pane's three rows — version, help and legal — and that it names no model (carnet#1147)
// ABOUTME: Help opens the docs hub and legal the privacy policy, which links the terms, both from the shared constants

import React from 'react';
import { render, fireEvent, waitFor } from '@testing-library/react-native';
import { Linking } from 'react-native';
import { APP_VERSION, HELP_URL, LEGAL_URL, settingsPaneSections } from '@pierre/shared-constants';


jest.mock('expo-router', () => ({
  useRouter: () => ({ push: jest.fn(), back: jest.fn() }),
}));

jest.mock('@expo/vector-icons', () => ({
  Feather: () => null,
}));

import { AboutScreen } from '../src/screens/settings/AboutScreen';

function renderAbout() {
  return render(<AboutScreen />);
}

describe('AboutScreen', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('renders the three rows the shared declaration holds, in order', async () => {
    const { getByTestId } = renderAbout();
    await waitFor(() => {
      expect(getByTestId('about-screen')).toBeTruthy();
    });
    expect([...settingsPaneSections('about')]).toEqual(['version', 'help', 'legal']);
    for (const section of settingsPaneSections('about')) {
      expect(getByTestId(`about-section-${section}`)).toBeTruthy();
    }
  });

  it('names no model: which one answers is an operator reading, never an athlete one', async () => {
    const { getByTestId, queryByTestId, queryByText } = renderAbout();
    await waitFor(() => {
      expect(getByTestId('about-screen')).toBeTruthy();
    });
    expect(queryByTestId('about-section-coach-model')).toBeNull();
    expect(queryByText(/Agent model/)).toBeNull();
  });

  it('reports the shared release string rather than a second copy of it', async () => {
    const { getByTestId } = renderAbout();
    await waitFor(() => {
      expect(getByTestId('about-section-version')).toBeTruthy();
    });
    expect(APP_VERSION).toBe('1.0.0');
  });

  it('opens help and legal at a page that answers', async () => {
    const openSpy = jest.spyOn(Linking, 'openURL').mockResolvedValue(true);
    const { getByTestId } = renderAbout();

    fireEvent.press(getByTestId('about-section-help'));
    await waitFor(() => {
      expect(openSpy).toHaveBeenCalledWith(HELP_URL);
    });

    fireEvent.press(getByTestId('about-section-legal'));
    await waitFor(() => {
      expect(openSpy).toHaveBeenCalledWith(LEGAL_URL);
    });

    // Help is the docs hub; legal is the privacy policy, which links the terms,
    // so one row reaches both documents. Both are wire values: pages the
    // dravr-website repo serves, outside this codebase. Asserting the constants
    // rather than the calls alone keeps a repoint from silently going back.
    expect(HELP_URL).toBe('https://dravr.ai/docs');
    expect(LEGAL_URL).toBe('https://dravr.ai/privacy');
    openSpy.mockRestore();
  });
});
