// ABOUTME: Unit tests for the mobile AI provider settings screen
// ABOUTME: Pins that a key is validated before it is saved, and that rejection blocks the save

import React from 'react';
import { render, fireEvent, waitFor } from '@testing-library/react-native';
import { Alert } from 'react-native';

jest.mock('expo-router', () => ({
  useRouter: () => ({ push: jest.fn(), back: jest.fn() }),
  useFocusEffect: (cb: () => void) => { require('react').useEffect(cb, []); },
}));

const mockGetLlmSettings = jest.fn();
const mockValidate = jest.fn();
const mockSave = jest.fn();
const mockDelete = jest.fn();
jest.mock('../src/services/api', () => ({
  userApi: {
    getLlmSettings: () => mockGetLlmSettings(),
    validateLlmCredentials: (d: unknown) => mockValidate(d),
    saveLlmCredentials: (d: unknown) => mockSave(d),
    deleteLlmCredentials: (p: string) => mockDelete(p),
  },
}));

import { LlmSettingsScreen } from '../src/screens/settings/LlmSettingsScreen';

const settings = {
  current_provider: 'gemini',
  providers: [
    { name: 'gemini', display_name: 'Gemini', has_credentials: true, credential_source: 'system', is_active: true },
    { name: 'openai', display_name: 'OpenAI', has_credentials: false, credential_source: null, is_active: false },
  ],
  user_credentials: [],
  tenant_credentials: [],
};

describe('LlmSettingsScreen', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockGetLlmSettings.mockResolvedValue(settings);
    mockValidate.mockResolvedValue({ valid: true });
    mockSave.mockResolvedValue({ success: true });
  });

  it('shows each provider and where its key comes from', async () => {
    const { getByTestId, getByText } = render(<LlmSettingsScreen />);
    await waitFor(() => expect(getByTestId('llm-provider-gemini')).toBeTruthy());
    expect(getByText('Key from system · Active')).toBeTruthy();
    expect(getByText('No key configured')).toBeTruthy();
  });

  it('validates a key before saving it', async () => {
    const alertSpy = jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
    const { getByTestId } = render(<LlmSettingsScreen />);
    await waitFor(() => expect(getByTestId('llm-select-openai')).toBeTruthy());

    fireEvent.press(getByTestId('llm-select-openai'));
    fireEvent.changeText(getByTestId('llm-api-key-input'), 'sk-test-key');
    fireEvent.press(getByTestId('llm-save-button'));

    await waitFor(() => {
      expect(mockValidate).toHaveBeenCalledWith({ provider: 'openai', api_key: 'sk-test-key' });
    });
    await waitFor(() => {
      expect(mockSave).toHaveBeenCalledWith({ provider: 'openai', api_key: 'sk-test-key' });
    });
    alertSpy.mockRestore();
  });

  it('does NOT save a key the provider rejects', async () => {
    // The whole point of validating first: an invalid key stored now surfaces
    // as a failed coaching turn later, far from the screen that caused it.
    const alertSpy = jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
    mockValidate.mockResolvedValueOnce({ valid: false, error: 'Invalid API key' });

    const { getByTestId } = render(<LlmSettingsScreen />);
    await waitFor(() => expect(getByTestId('llm-select-openai')).toBeTruthy());

    fireEvent.press(getByTestId('llm-select-openai'));
    fireEvent.changeText(getByTestId('llm-api-key-input'), 'sk-bad');
    fireEvent.press(getByTestId('llm-save-button'));

    await waitFor(() => {
      expect(alertSpy).toHaveBeenCalledWith('Key rejected', 'Invalid API key');
    });
    expect(mockSave).not.toHaveBeenCalled();
    alertSpy.mockRestore();
  });

  it('offers Remove only for a key the user owns', async () => {
    mockGetLlmSettings.mockResolvedValue({
      ...settings,
      user_credentials: [
        { id: 'c1', provider: 'openai', user_id: 'u1', created_at: 'now', updated_at: 'now' },
      ],
    });
    const { getByTestId, queryByTestId } = render(<LlmSettingsScreen />);
    await waitFor(() => expect(getByTestId('llm-remove-openai')).toBeTruthy());
    // Gemini's key comes from the system — the user cannot remove it.
    expect(queryByTestId('llm-remove-gemini')).toBeNull();
  });

  it('surfaces a load failure with a retry', async () => {
    mockGetLlmSettings.mockRejectedValueOnce(new Error('offline'));
    const { getByTestId } = render(<LlmSettingsScreen />);
    await waitFor(() => expect(getByTestId('llm-error')).toBeTruthy());
    expect(getByTestId('llm-retry')).toBeTruthy();
  });
});
