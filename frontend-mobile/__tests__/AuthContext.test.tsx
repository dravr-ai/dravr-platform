// ABOUTME: Unit tests for AuthContext
// ABOUTME: Tests session restore and renewal on launch, login, logout, and registration

import React from 'react';
import { render, waitFor, act, fireEvent } from '@testing-library/react-native';
import { AppState, Text, TouchableOpacity, type AppStateStatus } from 'react-native';
import { AuthProvider, useAuth } from '../src/contexts/AuthContext';
import { authApi, onAuthFailure } from '../src/services/api';

// Mock the api service
jest.mock('../src/services/api', () => ({
  authApi: {
    initializeAuth: jest.fn(),
    getStoredUser: jest.fn(),
    getSession: jest.fn(),
    login: jest.fn(),
    logout: jest.fn(),
    register: jest.fn(),
    storeAuth: jest.fn(),
  },
  onAuthFailure: jest.fn(() => jest.fn()),
}));

/** What the transport raises when the phone is offline: no response at all. */
const NETWORK_ERROR = Object.assign(new Error('Network Error'), { code: 'ERR_NETWORK' });

// Mock the firebase module to prevent WebBrowser initialization errors in tests
jest.mock('../src/firebase', () => ({
  signOutFromFirebase: jest.fn().mockResolvedValue(undefined),
  subscribeToAuthState: jest.fn(() => jest.fn()),
  getCurrentFirebaseUser: jest.fn().mockResolvedValue(null),
}));

// Test component that uses the auth context
function TestAuthConsumer() {
  const { user, isAuthenticated, isLoading, login, logout, register } = useAuth();

  return (
    <>
      <Text testID="loading">{isLoading ? 'loading' : 'loaded'}</Text>
      <Text testID="authenticated">{isAuthenticated ? 'authenticated' : 'not-authenticated'}</Text>
      <Text testID="user-email">{user?.email || 'no-user'}</Text>
      <TouchableOpacity
        testID="login-btn"
        onPress={() => login('test@example.com', 'password123')}
      >
        <Text>Login</Text>
      </TouchableOpacity>
      <TouchableOpacity
        testID="logout-btn"
        onPress={logout}
      >
        <Text>Logout</Text>
      </TouchableOpacity>
      <TouchableOpacity
        testID="register-btn"
        onPress={() => register('new@example.com', 'password123', 'New User')}
      >
        <Text>Register</Text>
      </TouchableOpacity>
    </>
  );
}

describe('AuthContext', () => {
  let warnSpy: jest.SpyInstance;
  // Captured for every test, not just the one that drives it: jest-expo's
  // AppState hands back no subscription, so the provider's foreground
  // listener would throw on unmount without a stand-in.
  let appStateListeners: ((status: AppStateStatus) => void)[] = [];
  let appStateSpy: jest.SpyInstance;

  beforeEach(() => {
    jest.clearAllMocks();
    // Unless a test says otherwise the phone is offline, so the launch-time
    // renewal fails on the transport and the stored session stands.
    (authApi.getSession as jest.Mock).mockRejectedValue(NETWORK_ERROR);
    warnSpy = jest.spyOn(console, 'warn').mockImplementation(() => {});
    appStateListeners = [];
    appStateSpy = jest
      .spyOn(AppState, 'addEventListener')
      .mockImplementation((_type, listener) => {
        appStateListeners.push(listener as (status: AppStateStatus) => void);
        return { remove: jest.fn() } as unknown as ReturnType<typeof AppState.addEventListener>;
      });
  });

  afterEach(() => {
    warnSpy.mockRestore();
    appStateSpy.mockRestore();
  });

  describe('useAuth hook', () => {
    it('should throw error when used outside AuthProvider', () => {
      const consoleError = jest.spyOn(console, 'error').mockImplementation(() => {});

      expect(() => {
        render(<TestAuthConsumer />);
      }).toThrow('useAuth must be used within an AuthProvider');

      consoleError.mockRestore();
    });
  });

  describe('AuthProvider initialization', () => {
    it('should start in loading state', async () => {
      (authApi.initializeAuth as jest.Mock).mockImplementation(
        () => new Promise(() => {}) // Never resolves - simulates loading
      );

      const { getByTestId } = render(
        <AuthProvider>
          <TestAuthConsumer />
        </AuthProvider>
      );

      expect(getByTestId('loading').children[0]).toBe('loading');
    });

    it('should initialize with stored user if token exists', async () => {
      const mockUser = {
        user_id: '123',
        email: 'stored@example.com',
        is_admin: false,
        role: 'user',
        user_status: 'active',
      };

      (authApi.initializeAuth as jest.Mock).mockResolvedValue(true);
      (authApi.getStoredUser as jest.Mock).mockResolvedValue(mockUser);

      const { getByTestId } = render(
        <AuthProvider>
          <TestAuthConsumer />
        </AuthProvider>
      );

      await waitFor(() => {
        expect(getByTestId('loading').children[0]).toBe('loaded');
      });

      expect(getByTestId('authenticated').children[0]).toBe('authenticated');
      expect(getByTestId('user-email').children[0]).toBe('stored@example.com');
    });

    it('should initialize as unauthenticated if no token', async () => {
      (authApi.initializeAuth as jest.Mock).mockResolvedValue(false);

      const { getByTestId } = render(
        <AuthProvider>
          <TestAuthConsumer />
        </AuthProvider>
      );

      await waitFor(() => {
        expect(getByTestId('loading').children[0]).toBe('loaded');
      });

      expect(getByTestId('authenticated').children[0]).toBe('not-authenticated');
      expect(getByTestId('user-email').children[0]).toBe('no-user');
    });

    it('should handle initialization error gracefully', async () => {
      (authApi.initializeAuth as jest.Mock).mockRejectedValue(new Error('Init failed'));
      const consoleSpy = jest.spyOn(console, 'error').mockImplementation(() => {});

      const { getByTestId } = render(
        <AuthProvider>
          <TestAuthConsumer />
        </AuthProvider>
      );

      await waitFor(() => {
        expect(getByTestId('loading').children[0]).toBe('loaded');
      });

      expect(getByTestId('authenticated').children[0]).toBe('not-authenticated');
      consoleSpy.mockRestore();
    });
  });

  // The server issues 24-hour tokens, and the shared API client exchanges the
  // refresh token behind this call once one has lapsed. These pin the
  // provider's half: that the trade happens on cold start and on foreground,
  // and what it does with the answer.
  describe('session restore on cold start', () => {
    const storedUser = {
      user_id: '123',
      email: 'stored@example.com',
      display_name: 'Stored Name',
      is_admin: false,
      role: 'user',
      user_status: 'active',
    };
    const renewedSession = {
      user: { ...storedUser, display_name: 'Renewed Name' },
      access_token: 'fresh-jwt',
      csrf_token: 'fresh-csrf',
    };

    function RestoreConsumer() {
      const { user, isAuthenticated, isLoading } = useAuth();
      return (
        <>
          <Text testID="loading">{isLoading ? 'loading' : 'loaded'}</Text>
          <Text testID="authenticated">{isAuthenticated ? 'authenticated' : 'not-authenticated'}</Text>
          <Text testID="display-name">{user?.display_name ?? 'no-user'}</Text>
        </>
      );
    }

    it('restores the stored session and persists the renewed token', async () => {
      (authApi.initializeAuth as jest.Mock).mockResolvedValue(true);
      (authApi.getStoredUser as jest.Mock).mockResolvedValue(storedUser);
      (authApi.getSession as jest.Mock).mockResolvedValue(renewedSession);
      (authApi.storeAuth as jest.Mock).mockResolvedValue(undefined);

      const { getByTestId } = render(
        <AuthProvider>
          <RestoreConsumer />
        </AuthProvider>
      );

      await waitFor(() => {
        expect(getByTestId('loading').children[0]).toBe('loaded');
      });
      expect(getByTestId('authenticated').children[0]).toBe('authenticated');

      // The renewed token is what the next cold start will read.
      await waitFor(() => {
        expect(authApi.storeAuth).toHaveBeenCalledWith('fresh-jwt', 'fresh-csrf', renewedSession.user);
      });
      expect(authApi.getSession).toHaveBeenCalledTimes(1);
      // And the server's view of the user replaces the stored copy.
      await waitFor(() => {
        expect(getByTestId('display-name').children[0]).toBe('Renewed Name');
      });
      expect(authApi.login).not.toHaveBeenCalled();
    });

    it('keeps the stored session when the renewal fails on the transport', async () => {
      (authApi.initializeAuth as jest.Mock).mockResolvedValue(true);
      (authApi.getStoredUser as jest.Mock).mockResolvedValue(storedUser);
      (authApi.getSession as jest.Mock).mockRejectedValue(NETWORK_ERROR);

      const { getByTestId } = render(
        <AuthProvider>
          <RestoreConsumer />
        </AuthProvider>
      );

      await waitFor(() => {
        expect(authApi.getSession).toHaveBeenCalledTimes(1);
      });
      expect(getByTestId('loading').children[0]).toBe('loaded');
      // Offline is not signed out: the stored user still gates the app stack.
      expect(getByTestId('authenticated').children[0]).toBe('authenticated');
      expect(getByTestId('display-name').children[0]).toBe('Stored Name');
      expect(authApi.storeAuth).not.toHaveBeenCalled();
    });

    it('does not ask the server for a session when nothing is stored', async () => {
      (authApi.initializeAuth as jest.Mock).mockResolvedValue(false);

      const { getByTestId } = render(
        <AuthProvider>
          <RestoreConsumer />
        </AuthProvider>
      );

      await waitFor(() => {
        expect(getByTestId('loading').children[0]).toBe('loaded');
      });
      expect(getByTestId('authenticated').children[0]).toBe('not-authenticated');
      expect(authApi.getSession).not.toHaveBeenCalled();
    });

    it('renews again when the app returns to the foreground', async () => {
      (authApi.initializeAuth as jest.Mock).mockResolvedValue(true);
      (authApi.getStoredUser as jest.Mock).mockResolvedValue(storedUser);
      (authApi.getSession as jest.Mock).mockResolvedValue(renewedSession);
      (authApi.storeAuth as jest.Mock).mockResolvedValue(undefined);

      const { getByTestId } = render(
        <AuthProvider>
          <RestoreConsumer />
        </AuthProvider>
      );

      await waitFor(() => {
        expect(getByTestId('display-name').children[0]).toBe('Renewed Name');
      });
      expect(authApi.getSession).toHaveBeenCalledTimes(1);
      expect(appStateListeners.length).toBeGreaterThan(0);

      // Backgrounding renews nothing; only the return to the foreground does.
      await act(async () => {
        appStateListeners.forEach((listener) => listener('background'));
      });
      expect(authApi.getSession).toHaveBeenCalledTimes(1);

      await act(async () => {
        appStateListeners.forEach((listener) => listener('active'));
      });
      await waitFor(() => {
        expect(authApi.getSession).toHaveBeenCalledTimes(2);
      });
      expect(authApi.storeAuth).toHaveBeenLastCalledWith('fresh-jwt', 'fresh-csrf', renewedSession.user);
    });

    it('does not subscribe to the foreground while signed out', async () => {
      (authApi.initializeAuth as jest.Mock).mockResolvedValue(false);

      const { getByTestId } = render(
        <AuthProvider>
          <RestoreConsumer />
        </AuthProvider>
      );

      await waitFor(() => {
        expect(getByTestId('loading').children[0]).toBe('loaded');
      });
      expect(appStateListeners).toHaveLength(0);
    });
  });

  describe('login', () => {
    it('should update state after successful login', async () => {
      const mockUser = {
        user_id: '123',
        email: 'test@example.com',
        is_admin: false,
        role: 'user',
        user_status: 'active',
      };

      const mockLoginResponse = {
        access_token: 'jwt-token',
        csrf_token: 'csrf-token',
        user: mockUser,
      };

      (authApi.initializeAuth as jest.Mock).mockResolvedValue(false);
      (authApi.login as jest.Mock).mockResolvedValue(mockLoginResponse);
      (authApi.storeAuth as jest.Mock).mockResolvedValue(undefined);

      const { getByTestId } = render(
        <AuthProvider>
          <TestAuthConsumer />
        </AuthProvider>
      );

      await waitFor(() => {
        expect(getByTestId('loading').children[0]).toBe('loaded');
      });

      // Initially not authenticated
      expect(getByTestId('authenticated').children[0]).toBe('not-authenticated');

      // Trigger login
      await act(async () => {
        fireEvent.press(getByTestId('login-btn'));
      });

      await waitFor(() => {
        expect(getByTestId('authenticated').children[0]).toBe('authenticated');
      });

      expect(authApi.login).toHaveBeenCalledWith({
        email: 'test@example.com',
        password: 'password123',
      });
      expect(authApi.storeAuth).toHaveBeenCalledWith('jwt-token', 'csrf-token', mockUser);
    });
  });

  describe('logout', () => {
    it('should clear user state after logout', async () => {
      const mockUser = {
        user_id: '123',
        email: 'stored@example.com',
        is_admin: false,
        role: 'user',
        user_status: 'active',
      };

      (authApi.initializeAuth as jest.Mock).mockResolvedValue(true);
      (authApi.getStoredUser as jest.Mock).mockResolvedValue(mockUser);
      (authApi.logout as jest.Mock).mockResolvedValue(undefined);

      const { getByTestId } = render(
        <AuthProvider>
          <TestAuthConsumer />
        </AuthProvider>
      );

      await waitFor(() => {
        expect(getByTestId('authenticated').children[0]).toBe('authenticated');
      });

      // Trigger logout
      await act(async () => {
        fireEvent.press(getByTestId('logout-btn'));
      });

      await waitFor(() => {
        expect(getByTestId('authenticated').children[0]).toBe('not-authenticated');
      });

      expect(authApi.logout).toHaveBeenCalled();
    });
  });

  describe('register', () => {
    it('should call register API', async () => {
      (authApi.initializeAuth as jest.Mock).mockResolvedValue(false);
      (authApi.register as jest.Mock).mockResolvedValue({ success: true });

      const { getByTestId } = render(
        <AuthProvider>
          <TestAuthConsumer />
        </AuthProvider>
      );

      await waitFor(() => {
        expect(getByTestId('loading').children[0]).toBe('loaded');
      });

      // Trigger register
      await act(async () => {
        fireEvent.press(getByTestId('register-btn'));
      });

      expect(authApi.register).toHaveBeenCalledWith({
        email: 'new@example.com',
        password: 'password123',
        display_name: 'New User',
      });
    });
  });

  describe('isAuthenticated', () => {
    it('should be false for pending users', async () => {
      const mockPendingUser = {
        user_id: '123',
        email: 'pending@example.com',
        is_admin: false,
        role: 'user',
        user_status: 'pending', // Not active
      };

      (authApi.initializeAuth as jest.Mock).mockResolvedValue(true);
      (authApi.getStoredUser as jest.Mock).mockResolvedValue(mockPendingUser);

      const { getByTestId } = render(
        <AuthProvider>
          <TestAuthConsumer />
        </AuthProvider>
      );

      await waitFor(() => {
        expect(getByTestId('loading').children[0]).toBe('loaded');
      });

      // User exists but isAuthenticated should be false due to pending status
      expect(getByTestId('user-email').children[0]).toBe('pending@example.com');
      expect(getByTestId('authenticated').children[0]).toBe('not-authenticated');
    });

    it('should be true for active users', async () => {
      const mockActiveUser = {
        user_id: '123',
        email: 'active@example.com',
        is_admin: false,
        role: 'user',
        user_status: 'active',
      };

      (authApi.initializeAuth as jest.Mock).mockResolvedValue(true);
      (authApi.getStoredUser as jest.Mock).mockResolvedValue(mockActiveUser);

      const { getByTestId } = render(
        <AuthProvider>
          <TestAuthConsumer />
        </AuthProvider>
      );

      await waitFor(() => {
        expect(getByTestId('loading').children[0]).toBe('loaded');
      });

      expect(getByTestId('authenticated').children[0]).toBe('authenticated');
    });
  });

  describe('onAuthFailure listener', () => {
    it('should register auth failure listener on mount', async () => {
      (authApi.initializeAuth as jest.Mock).mockResolvedValue(false);

      render(
        <AuthProvider>
          <TestAuthConsumer />
        </AuthProvider>
      );

      expect(onAuthFailure).toHaveBeenCalled();
    });

    // The registration is only half the contract. The shared response
    // interceptor clears the stored token and then fires this listener; if the
    // listener does not drop the in-memory user, the app keeps rendering signed-in
    // chrome over a session the server has already refused.
    it('should drop the signed-in user when the listener fires', async () => {
      const mockUser = {
        user_id: '123',
        email: 'stored@example.com',
        is_admin: false,
        role: 'user',
        user_status: 'active',
      };

      (authApi.initializeAuth as jest.Mock).mockResolvedValue(true);
      (authApi.getStoredUser as jest.Mock).mockResolvedValue(mockUser);

      const { getByTestId } = render(
        <AuthProvider>
          <TestAuthConsumer />
        </AuthProvider>
      );

      await waitFor(() => {
        expect(getByTestId('authenticated').children[0]).toBe('authenticated');
      });

      const notifyAuthFailure = (onAuthFailure as jest.Mock).mock.calls[0][0] as () => void;

      await act(async () => {
        notifyAuthFailure();
      });

      expect(getByTestId('authenticated').children[0]).toBe('not-authenticated');
      expect(getByTestId('user-email').children[0]).toBe('no-user');
    });
  });
});
