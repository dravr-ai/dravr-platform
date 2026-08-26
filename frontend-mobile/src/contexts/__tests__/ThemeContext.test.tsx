// ABOUTME: Tests for ThemeContext's server sync — every setPref writes users.theme
// ABOUTME: Verifies light/dark send themselves, System sends null, and a failed write only toasts

import React from 'react';
import { Text, TouchableOpacity } from 'react-native';
import { render, fireEvent, waitFor } from '@testing-library/react-native';
import Toast from 'react-native-toast-message';
import { ThemeProvider, useTheme } from '../ThemeContext';
import { userApi } from '../../services/api';

jest.mock('nativewind', () => ({
  useColorScheme: () => ({ colorScheme: 'dark', setColorScheme: jest.fn() }),
}));

jest.mock('../../services/api', () => ({
  userApi: {
    updateTheme: jest.fn().mockResolvedValue(undefined),
  },
}));

function Harness() {
  const { pref, setPref } = useTheme();
  return (
    <>
      <Text testID="current-pref">{pref}</Text>
      <TouchableOpacity testID="pick-light" onPress={() => void setPref('light')}>
        <Text>Light</Text>
      </TouchableOpacity>
      <TouchableOpacity testID="pick-dark" onPress={() => void setPref('dark')}>
        <Text>Dark</Text>
      </TouchableOpacity>
      <TouchableOpacity testID="pick-system" onPress={() => void setPref('system')}>
        <Text>System</Text>
      </TouchableOpacity>
    </>
  );
}

function renderHarness() {
  return render(
    <ThemeProvider>
      <Harness />
    </ThemeProvider>,
  );
}

describe('ThemeContext server sync', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    jest.mocked(userApi.updateTheme).mockResolvedValue(undefined);
  });

  it('sends the picked scheme to PUT /api/user/theme', async () => {
    const screen = renderHarness();

    fireEvent.press(screen.getByTestId('pick-light'));
    await waitFor(() => {
      expect(userApi.updateTheme).toHaveBeenCalledWith('light');
    });

    fireEvent.press(screen.getByTestId('pick-dark'));
    await waitFor(() => {
      expect(userApi.updateTheme).toHaveBeenCalledWith('dark');
    });
    expect(userApi.updateTheme).toHaveBeenCalledTimes(2);
  });

  it('sends null when the user picks System — no pin, follow the device', async () => {
    const screen = renderHarness();

    fireEvent.press(screen.getByTestId('pick-system'));

    await waitFor(() => {
      expect(userApi.updateTheme).toHaveBeenCalledTimes(1);
    });
    expect(userApi.updateTheme).toHaveBeenCalledWith(null);
  });

  it('keeps the local preference and toasts when the server write fails', async () => {
    jest.mocked(userApi.updateTheme).mockRejectedValueOnce(new Error('offline'));
    const screen = renderHarness();

    fireEvent.press(screen.getByTestId('pick-light'));

    // The local flip is never blocked by the failed write…
    await waitFor(() => {
      expect(screen.getByTestId('current-pref')).toHaveTextContent('light');
    });
    // …and the failure surfaces only as an error toast.
    await waitFor(() => {
      expect(Toast.show).toHaveBeenCalledWith({
        type: 'error',
        text1: 'Thème',
        text2: "Le thème a changé ici, mais la préférence n'a pas pu être enregistrée sur ton compte. Réessaie.",
      });
    });
  });
});
