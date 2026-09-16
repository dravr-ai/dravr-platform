// ABOUTME: Authentication context provider for Dravr Mobile app
// ABOUTME: Restores the stored session on launch, renews its token against the server, and owns login/logout

import React, { createContext, useContext, useState, useEffect, useCallback, useMemo, type ReactNode } from 'react';
import { AppState, type AppStateStatus } from 'react-native';
import { authApi, onAuthFailure, userApi } from '../services/api';
import { signOutFromFirebase } from '../firebase';
import type { User, FirebaseLoginResponse } from '../types';

interface AuthContextType {
  user: User | null;
  isAuthenticated: boolean;
  isLoading: boolean;
  login: (email: string, password: string) => Promise<void>;
  loginWithFirebase: (idToken: string) => Promise<FirebaseLoginResponse>;
  logout: () => Promise<void>;
  register: (email: string, password: string, displayName?: string) => Promise<void>;
  updateUser: (patch: Partial<User>) => Promise<void>;
}

const AuthContext = createContext<AuthContextType | undefined>(undefined);

interface AuthProviderProps {
  children: ReactNode;
}

/**
 * Read the device's IANA timezone via `Intl.DateTimeFormat` and PUT it
 * to the server so the chat prompt can render `{{CURRENT_DATE}}` in
 * the user's local calendar day. Best-effort: errors are swallowed so
 * login flows never break on a transient PUT failure.
 */
async function captureUserTimezone(): Promise<void> {
  try {
    const tz = Intl.DateTimeFormat().resolvedOptions().timeZone;
    if (tz) {
      await userApi.setTimezone(tz);
    }
  } catch (err) {
    console.warn('Failed to persist user timezone:', err);
  }
}

/**
 * Trade the stored JWT for a fresh one and persist it.
 *
 * The server issues 24-hour tokens (`JWT_EXPIRY_HOURS`) and no refresh token,
 * so a stored session only outlives a day if the app renews it while the
 * token is still valid. `GET /api/auth/session` accepts the bearer and answers
 * with a new token, user and CSRF token — the same call the web client makes
 * on every page load, which is why a browser session slides and a phone
 * session used to end at the first request after the 24-hour mark.
 *
 * Resolves to the server's view of the user, or null when the renewal did not
 * complete: the phone must open offline and Cloud Run cold starts take
 * seconds, so a transport failure keeps the stored session as it is. A 401
 * needs no handling here — the shared response interceptor has already
 * cleared storage and fired `onAuthFailure`, which drops the user below.
 */
async function renewSession(): Promise<User | null> {
  try {
    const session = await authApi.getSession();
    await authApi.storeAuth(session.access_token, session.csrf_token, session.user);
    return session.user;
  } catch (err) {
    console.warn('Session renewal failed; keeping the stored session:', err);
    return null;
  }
}

export function AuthProvider({ children }: AuthProviderProps) {
  const [user, setUser] = useState<User | null>(null);
  const [isLoading, setIsLoading] = useState(true);

  // Restore the session from storage, then renew it. The stored user gates the
  // navigator as soon as it is read — renewal happens after `isLoading` drops,
  // so a cold start never waits on the network.
  useEffect(() => {
    let cancelled = false;
    const initAuth = async () => {
      let storedUser: User | null = null;
      try {
        const hasToken = await authApi.initializeAuth();
        if (hasToken) {
          storedUser = await authApi.getStoredUser();
          if (storedUser) {
            setUser(storedUser);
          }
        }
      } catch (error) {
        console.error('Failed to initialize auth:', error);
      } finally {
        setIsLoading(false);
      }
      if (!storedUser) return;
      const renewed = await renewSession();
      if (renewed && !cancelled) {
        setUser(renewed);
      }
    };

    initAuth();
    return () => {
      cancelled = true;
    };
  }, []);

  // A phone can sit in the background for days without a cold start, so the
  // launch-time renewal alone would let the token lapse. Renew again whenever
  // the app returns to the foreground while signed in. Keyed on the boolean,
  // not the user object, so a renewal does not re-subscribe.
  const signedIn = user !== null;
  useEffect(() => {
    if (!signedIn) return undefined;
    const subscription = AppState.addEventListener('change', (state: AppStateStatus) => {
      if (state !== 'active') return;
      void renewSession().then((renewed) => {
        if (renewed) {
          setUser(renewed);
        }
      });
    });
    return () => subscription.remove();
  }, [signedIn]);

  // Listen for auth failures (401 responses)
  useEffect(() => {
    const unsubscribe = onAuthFailure(() => {
      setUser(null);
    });
    return unsubscribe;
  }, []);

  const login = useCallback(async (email: string, password: string) => {
    const response = await authApi.login({ email, password });

    // OAuth2 response contains access_token and user info
    const loginUser: User = response.user || {
      user_id: '',
      email,
      is_admin: false,
      role: 'user',
      user_status: 'active',
    };

    await authApi.storeAuth(response.access_token, response.csrf_token || '', loginUser);
    setUser(loginUser);

    // Best-effort: persist the device's IANA timezone so the chat
    // prompt resolves {{CURRENT_DATE}} in the user's local calendar.
    // Failures don't block login — UTC fallback keeps chat usable.
    void captureUserTimezone();
  }, []);

  const loginWithFirebase = useCallback(async (idToken: string): Promise<FirebaseLoginResponse> => {
    const response = await authApi.loginWithFirebase({ idToken });

    // Store auth tokens and user info
    await authApi.storeAuth(response.jwt_token, response.csrf_token, response.user);
    setUser(response.user);

    void captureUserTimezone();

    return response;
  }, []);

  const logout = useCallback(async () => {
    try {
      await authApi.logout();
    } catch (error) {
      // Server call may fail (e.g. network error via tunnel),
      // but authApi.logout() already clears local storage in its finally block.
      console.warn('Server logout request failed:', error);
    }
    await signOutFromFirebase();
    setUser(null);
  }, []);

  const register = useCallback(async (email: string, password: string, displayName?: string) => {
    await authApi.register({ email, password, display_name: displayName });
    // After registration, user needs to log in (or wait for approval if pending)
  }, []);

  const updateUser = useCallback(async (patch: Partial<User>) => {
    let next: User | null = null;
    setUser((current) => {
      if (!current) return current;
      next = { ...current, ...patch };
      return next;
    });
    if (next) {
      await authApi.storeUser(next);
    }
  }, []);

  const isAuthenticated = !!user && user.user_status === 'active';

  // Memoize context value to prevent unnecessary re-renders of consumers
  const value: AuthContextType = useMemo(() => ({
    user,
    isAuthenticated,
    isLoading,
    login,
    loginWithFirebase,
    logout,
    register,
    updateUser,
  }), [user, isAuthenticated, isLoading, login, loginWithFirebase, logout, register, updateUser]);

  return (
    <AuthContext.Provider value={value}>
      {children}
    </AuthContext.Provider>
  );
}

export function useAuth(): AuthContextType {
  const context = useContext(AuthContext);
  if (context === undefined) {
    throw new Error('useAuth must be used within an AuthProvider');
  }
  return context;
}

export { AuthContext };
