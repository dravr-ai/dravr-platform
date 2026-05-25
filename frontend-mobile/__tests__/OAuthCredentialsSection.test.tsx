// ABOUTME: Unit tests for OAuthCredentialsSection component
// ABOUTME: Tests OAuth credential management UI including add, delete, and display

import React from 'react';
import { render, waitFor, act, fireEvent } from '@testing-library/react-native';
import { Alert } from 'react-native';
import { OAuthCredentialsSection } from '../src/components/OAuthCredentialsSection';
import { userApi } from '../src/services/api';

// Mock the api service
jest.mock('../src/services/api', () => ({
  userApi: {
    getUserOAuthApps: jest.fn(),
    registerUserOAuthApp: jest.fn(),
    deleteUserOAuthApp: jest.fn(),
  },
}));

// Mock Alert
jest.spyOn(Alert, 'alert');

describe('OAuthCredentialsSection', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  describe('initial render', () => {
    it('should show loading state initially', async () => {
      (userApi.getUserOAuthApps as jest.Mock).mockImplementation(
        () => new Promise(() => {}) // Never resolves - simulates loading
      );

      const { getByTestId } = render(<OAuthCredentialsSection />);

      // Component shows ActivityIndicator during loading
      // We can verify the API was called
      expect(userApi.getUserOAuthApps).toHaveBeenCalled();
    });

    it('should show empty state when no OAuth apps configured', async () => {
      (userApi.getUserOAuthApps as jest.Mock).mockResolvedValue({ apps: [] });

      const { getByText } = render(<OAuthCredentialsSection />);

      await waitFor(() => {
        expect(getByText('No custom OAuth credentials configured')).toBeTruthy();
      });
    });

    it('should display configured OAuth apps', async () => {
      const mockApps = [
        {
          provider: 'whoop',
          client_id: '12345678',
          redirect_uri: 'https://pierre.fit/api/oauth/callback/whoop',
          created_at: '2024-01-01T00:00:00Z',
        },
      ];
      (userApi.getUserOAuthApps as jest.Mock).mockResolvedValue({ apps: mockApps });

      const { getByText } = render(<OAuthCredentialsSection />);

      await waitFor(() => {
        expect(getByText('WHOOP')).toBeTruthy();
        expect(getByText('Configured')).toBeTruthy();
        expect(getByText('Client ID: 12345678')).toBeTruthy();
      });
    });

    it('should mask long client IDs', async () => {
      const mockApps = [
        {
          provider: 'whoop',
          client_id: '1234567890abcdef',
          redirect_uri: 'https://pierre.fit/api/oauth/callback/whoop',
          created_at: '2024-01-01T00:00:00Z',
        },
      ];
      (userApi.getUserOAuthApps as jest.Mock).mockResolvedValue({ apps: mockApps });

      const { getByText } = render(<OAuthCredentialsSection />);

      await waitFor(() => {
        expect(getByText('Client ID: 12345678...')).toBeTruthy();
      });
    });
  });

  describe('add button visibility', () => {
    it('should show add button when providers are available', async () => {
      (userApi.getUserOAuthApps as jest.Mock).mockResolvedValue({ apps: [] });

      const { getByText } = render(<OAuthCredentialsSection />);

      await waitFor(() => {
        expect(getByText('+ Add')).toBeTruthy();
      });
    });

    it('should hide add button when all providers are configured', async () => {
      // After the 2026-Q2 provider cleanup, BYO-OAuth-app is WHOOP-only.
      const allProviders = [
        { provider: 'whoop', client_id: '1', redirect_uri: '', created_at: '' },
      ];
      (userApi.getUserOAuthApps as jest.Mock).mockResolvedValue({ apps: allProviders });

      const { queryByText } = render(<OAuthCredentialsSection />);

      await waitFor(() => {
        expect(queryByText('+ Add')).toBeNull();
      });
    });
  });

  describe('add modal', () => {
    it('should open add modal when add button is pressed', async () => {
      (userApi.getUserOAuthApps as jest.Mock).mockResolvedValue({ apps: [] });

      const { getByText } = render(<OAuthCredentialsSection />);

      await waitFor(() => {
        expect(getByText('+ Add')).toBeTruthy();
      });

      fireEvent.press(getByText('+ Add'));

      await waitFor(() => {
        expect(getByText('Add OAuth Credentials')).toBeTruthy();
        expect(getByText('Select a provider...')).toBeTruthy();
      });
    });

    it('should close modal when cancel is pressed', async () => {
      (userApi.getUserOAuthApps as jest.Mock).mockResolvedValue({ apps: [] });

      const { getByText, queryByText } = render(<OAuthCredentialsSection />);

      await waitFor(() => {
        fireEvent.press(getByText('+ Add'));
      });

      await waitFor(() => {
        expect(getByText('Add OAuth Credentials')).toBeTruthy();
      });

      fireEvent.press(getByText('Cancel'));

      await waitFor(() => {
        expect(queryByText('Add OAuth Credentials')).toBeNull();
      });
    });
  });

  describe('form validation', () => {
    it('should show error when saving without selecting provider', async () => {
      (userApi.getUserOAuthApps as jest.Mock).mockResolvedValue({ apps: [] });

      const { getByText } = render(<OAuthCredentialsSection />);

      await waitFor(() => {
        fireEvent.press(getByText('+ Add'));
      });

      await waitFor(() => {
        expect(getByText('Add OAuth Credentials')).toBeTruthy();
      });

      fireEvent.press(getByText('Save'));

      expect(Alert.alert).toHaveBeenCalledWith('Error', 'Please select a provider');
    });

    it('should show error when saving without client ID', async () => {
      (userApi.getUserOAuthApps as jest.Mock).mockResolvedValue({ apps: [] });

      const { getByText, getByPlaceholderText } = render(<OAuthCredentialsSection />);

      await waitFor(() => {
        fireEvent.press(getByText('+ Add'));
      });

      // Open provider picker
      await waitFor(() => {
        fireEvent.press(getByText('Select a provider...'));
      });

      // Select WHOOP — BYO-OAuth-app is WHOOP-only after the 2026-Q2 cleanup.
      await waitFor(() => {
        fireEvent.press(getByText('WHOOP'));
      });

      // Try to save without client ID
      fireEvent.press(getByText('Save'));

      expect(Alert.alert).toHaveBeenCalledWith('Error', 'Please enter a Client ID');
    });

    it('should show error when saving without client secret', async () => {
      (userApi.getUserOAuthApps as jest.Mock).mockResolvedValue({ apps: [] });

      const { getByText, getByPlaceholderText } = render(<OAuthCredentialsSection />);

      await waitFor(() => {
        fireEvent.press(getByText('+ Add'));
      });

      // Open provider picker
      await waitFor(() => {
        fireEvent.press(getByText('Select a provider...'));
      });

      // Select WHOOP
      await waitFor(() => {
        fireEvent.press(getByText('WHOOP'));
      });

      // Enter client ID but not secret
      fireEvent.changeText(getByPlaceholderText('Enter your OAuth client ID'), 'my-client-id');

      fireEvent.press(getByText('Save'));

      expect(Alert.alert).toHaveBeenCalledWith('Error', 'Please enter a Client Secret');
    });
  });

  describe('save credentials', () => {
    it('should save credentials successfully', async () => {
      (userApi.getUserOAuthApps as jest.Mock).mockResolvedValue({ apps: [] });
      (userApi.registerUserOAuthApp as jest.Mock).mockResolvedValue({
        success: true,
        provider: 'whoop',
        message: 'Credentials saved',
      });

      const { getByText, getByPlaceholderText, queryByText } = render(<OAuthCredentialsSection />);

      await waitFor(() => {
        fireEvent.press(getByText('+ Add'));
      });

      // Open provider picker and select WHOOP
      await waitFor(() => {
        fireEvent.press(getByText('Select a provider...'));
      });

      await waitFor(() => {
        fireEvent.press(getByText('WHOOP'));
      });

      // Fill in the form
      fireEvent.changeText(getByPlaceholderText('Enter your OAuth client ID'), 'my-client-id');
      fireEvent.changeText(getByPlaceholderText('Enter your OAuth client secret'), 'my-client-secret');

      // Save
      await act(async () => {
        fireEvent.press(getByText('Save'));
      });

      expect(userApi.registerUserOAuthApp).toHaveBeenCalledWith({
        provider: 'whoop',
        client_id: 'my-client-id',
        client_secret: 'my-client-secret',
        redirect_uri: 'https://pierre.fit/api/oauth/callback/whoop',
      });

      await waitFor(() => {
        expect(Alert.alert).toHaveBeenCalledWith('Success', 'WHOOP credentials saved successfully');
      });
    });

    it('should handle save error', async () => {
      (userApi.getUserOAuthApps as jest.Mock).mockResolvedValue({ apps: [] });
      (userApi.registerUserOAuthApp as jest.Mock).mockRejectedValue(
        new Error('Invalid credentials')
      );

      const { getByText, getByPlaceholderText } = render(<OAuthCredentialsSection />);

      await waitFor(() => {
        fireEvent.press(getByText('+ Add'));
      });

      // Select provider
      await waitFor(() => {
        fireEvent.press(getByText('Select a provider...'));
      });

      await waitFor(() => {
        fireEvent.press(getByText('WHOOP'));
      });

      // Fill in the form
      fireEvent.changeText(getByPlaceholderText('Enter your OAuth client ID'), 'my-client-id');
      fireEvent.changeText(getByPlaceholderText('Enter your OAuth client secret'), 'my-client-secret');

      // Save
      await act(async () => {
        fireEvent.press(getByText('Save'));
      });

      await waitFor(() => {
        expect(Alert.alert).toHaveBeenCalledWith('Error', 'Invalid credentials');
      });
    });
  });

  describe('delete credentials', () => {
    it('should show confirmation dialog when remove is pressed', async () => {
      const mockApps = [
        {
          provider: 'whoop',
          client_id: '12345678',
          redirect_uri: 'https://pierre.fit/api/oauth/callback/whoop',
          created_at: '2024-01-01T00:00:00Z',
        },
      ];
      (userApi.getUserOAuthApps as jest.Mock).mockResolvedValue({ apps: mockApps });

      const { getByText } = render(<OAuthCredentialsSection />);

      await waitFor(() => {
        expect(getByText('Remove')).toBeTruthy();
      });

      fireEvent.press(getByText('Remove'));

      expect(Alert.alert).toHaveBeenCalledWith(
        'Remove Credentials',
        expect.stringContaining('WHOOP'),
        expect.any(Array)
      );
    });

    it('should delete credentials when confirmed', async () => {
      const mockApps = [
        {
          provider: 'whoop',
          client_id: '12345678',
          redirect_uri: 'https://pierre.fit/api/oauth/callback/whoop',
          created_at: '2024-01-01T00:00:00Z',
        },
      ];
      (userApi.getUserOAuthApps as jest.Mock).mockResolvedValue({ apps: mockApps });
      (userApi.deleteUserOAuthApp as jest.Mock).mockResolvedValue(undefined);

      // Capture the Alert.alert mock to simulate pressing "Remove"
      let deleteCallback: (() => void) | undefined;
      (Alert.alert as jest.Mock).mockImplementation((title, message, buttons) => {
        const removeButton = buttons?.find((b: { text: string }) => b.text === 'Remove');
        if (removeButton?.onPress) {
          deleteCallback = removeButton.onPress;
        }
      });

      const { getByText } = render(<OAuthCredentialsSection />);

      await waitFor(() => {
        expect(getByText('Remove')).toBeTruthy();
      });

      fireEvent.press(getByText('Remove'));

      // Simulate confirming the delete
      if (deleteCallback) {
        await act(async () => {
          deleteCallback!();
        });
      }

      expect(userApi.deleteUserOAuthApp).toHaveBeenCalledWith('whoop');
    });

    it('should not delete when cancel is pressed', async () => {
      const mockApps = [
        {
          provider: 'whoop',
          client_id: '12345678',
          redirect_uri: 'https://pierre.fit/api/oauth/callback/whoop',
          created_at: '2024-01-01T00:00:00Z',
        },
      ];
      (userApi.getUserOAuthApps as jest.Mock).mockResolvedValue({ apps: mockApps });

      // Capture the Alert.alert mock to verify Cancel doesn't call delete
      (Alert.alert as jest.Mock).mockImplementation((title, message, buttons) => {
        // Don't call any button callback - simulates pressing Cancel
      });

      const { getByText } = render(<OAuthCredentialsSection />);

      await waitFor(() => {
        expect(getByText('Remove')).toBeTruthy();
      });

      fireEvent.press(getByText('Remove'));

      expect(userApi.deleteUserOAuthApp).not.toHaveBeenCalled();
    });
  });

  describe('provider picker', () => {
    it('should show WHOOP in picker when no providers are configured', async () => {
      // After the 2026-Q2 provider cleanup, BYO-OAuth-app is WHOOP-only.
      (userApi.getUserOAuthApps as jest.Mock).mockResolvedValue({ apps: [] });

      const { getByText } = render(<OAuthCredentialsSection />);

      await waitFor(() => {
        fireEvent.press(getByText('+ Add'));
      });

      // Open provider picker
      await waitFor(() => {
        fireEvent.press(getByText('Select a provider...'));
      });

      await waitFor(() => {
        expect(getByText('WHOOP')).toBeTruthy();
      });
    });

    it('should update redirect URI when provider is selected', async () => {
      (userApi.getUserOAuthApps as jest.Mock).mockResolvedValue({ apps: [] });

      const { getByText } = render(<OAuthCredentialsSection />);

      await waitFor(() => {
        fireEvent.press(getByText('+ Add'));
      });

      // Open provider picker
      await waitFor(() => {
        fireEvent.press(getByText('Select a provider...'));
      });

      // Select WHOOP
      await waitFor(() => {
        fireEvent.press(getByText('WHOOP'));
      });

      // Check that redirect URI was updated (displayed as Text, not Input)
      await waitFor(() => {
        expect(getByText('https://pierre.fit/api/oauth/callback/whoop')).toBeTruthy();
      });
    });
  });

  describe('error handling', () => {
    it('should handle API error when loading OAuth apps', async () => {
      const consoleSpy = jest.spyOn(console, 'error').mockImplementation(() => {});
      (userApi.getUserOAuthApps as jest.Mock).mockRejectedValue(new Error('Network error'));

      const { getByText } = render(<OAuthCredentialsSection />);

      await waitFor(() => {
        expect(getByText('No custom OAuth credentials configured')).toBeTruthy();
      });

      consoleSpy.mockRestore();
    });
  });
});
