// ABOUTME: Authentication context provider for Dravr Mobile app
// ABOUTME: Restores the stored session on launch, renews its token against the server, and owns sign-in/logout

import React, { createContext, useContext, useState, useEffect, useCallback, useMemo, type ReactNode } from 'react';
import { AppState, type AppStateStatus } from 'react-native';
import { authApi, onAuthFailure, userApi } from '../services/api';
import { signOutFromFirebase } from '../firebase';
import { forgetLanguageChoiceNative } from '@pierre/i18n/native';
import * as Linking from 'expo-linking';
import type { RegistrationSignIn } from '@pierre/api-client';
import {
  SIGN_IN_CALLBACK_PATH,
  expoPkceCrypto,
  redeemCode,
  signInWithHostedPage,
} from '../utils/hostedSignIn';
import { releasePushDevice, unregisterPushDevice } from '../services/pushNotifications';
import type { User, FirebaseLoginResponse, LoginResponse, RegisterResponse } from '../types';

/**
 * What a registration ended in: the server's response, and whether the
 * athlete is now signed in — the account was active at once and its
 * sign-in code was redeemed (carnet#1147).
 */
export interface RegistrationOutcome {
  response: RegisterResponse;
  signedIn: boolean;
}

interface AuthContextType {
  user: User | null;
  isAuthenticated: boolean;
  isLoading: boolean;
  /**
   * Sign in on the server's hosted login page (carnet#787). Resolves `true`
   * once signed in, `false` when the athlete closed the browser; throws when
   * the server refused the sign-in or the code could not be redeemed.
   */
  signIn: () => Promise<boolean>;
  loginWithFirebase: (idToken: string) => Promise<FirebaseLoginResponse>;
  logout: () => Promise<void>;
  /**
   * Create an account. Resolves with the server's response, whose `message`
   * says what happens next, and whether the athlete is now signed in: an
   * account active at once comes back with a sign-in code, redeemed here.
   * A code that could not be redeemed resolves `signedIn: false`, never an
   * error — the account exists. `turnstileToken` is the bot-check token when
   * the sign-up config names a Turnstile site key.
   */
  register: (
    email: string,
    password: string,
    displayName?: string,
    turnstileToken?: string,
    signupCode?: string,
  ) => Promise<RegistrationOutcome>;
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
 * The server issues 24-hour tokens (`JWT_EXPIRY_HOURS`). While the stored one
 * is still valid, `GET /api/auth/session` accepts it and answers with a new
 * token, user and CSRF token — the same call the web client makes on every
 * page load, which is why a browser session slides. Once it has lapsed, the
 * shared API client exchanges the refresh token the login stored in the
 * keychain and retries this same call, so a phone closed for a week opens
 * signed in; only a refresh token the server no longer honours (a month
 * unused, a logout, a password change) ends here on the login screen.
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

  // Listen for auth failures (401 responses). The session is already gone, so
  // the phone releases its push token itself rather than asking the server.
  useEffect(() => {
    const unsubscribe = onAuthFailure(() => {
      setUser(null);
      void releasePushDevice();
      // Ends the session as Sign out does, so it forgets this phone's language
      // choice too: the next sign-in may be another account (web's 401 goes
      // through logout() and already does).
      void forgetLanguageChoiceNative();
    });
    return unsubscribe;
  }, []);

  // Take a redeemed sign-in as this phone's session: the hosted sign-in and a
  // registration that came back with a code both end here.
  const adoptSession = useCallback(async (response: LoginResponse): Promise<void> => {
    await authApi.storeAuth(response.access_token, response.csrf_token || '', response.user);
    setUser(response.user);

    // Best-effort: persist the device's IANA timezone so the chat
    // prompt resolves {{CURRENT_DATE}} in the user's local calendar.
    // Failures don't block sign-in — UTC fallback keeps chat usable.
    void captureUserTimezone();
  }, []);

  // The sign-in lives here rather than in the login screen so that it
  // completes even if the screen remounts while the browser is open: on
  // Android the redirect also reaches the router as a deep link.
  const signIn = useCallback(async (): Promise<boolean> => {
    const response = await signInWithHostedPage();
    if (!response) {
      return false;
    }
    await adoptSession(response);
    return true;
  }, [adoptSession]);

  const loginWithFirebase = useCallback(async (idToken: string): Promise<FirebaseLoginResponse> => {
    const response = await authApi.loginWithFirebase({ idToken });

    // Store auth tokens and user info
    await authApi.storeAuth(response.jwt_token, response.csrf_token, response.user);
    setUser(response.user);

    void captureUserTimezone();

    return response;
  }, []);

  const logout = useCallback(async () => {
    // Before the server logout: deactivating this phone's push device needs
    // the session that logout ends. It never throws, and gives up on an
    // unreachable server after a few seconds.
    await unregisterPushDevice();
    try {
      await authApi.logout();
    } catch (error) {
      // Server call may fail (e.g. network error via tunnel),
      // but authApi.logout() already clears local storage in its finally block.
      console.warn('Server logout request failed:', error);
    }
    await signOutFromFirebase();
    // The language picked on this phone belonged to this account; left behind
    // it would be written onto the next account signed in here.
    await forgetLanguageChoiceNative();
    setUser(null);
  }, []);

  const register = useCallback(async (
    email: string,
    password: string,
    displayName?: string,
    turnstileToken?: string,
    signupCode?: string,
  ): Promise<RegistrationOutcome> => {
    // A fresh PKCE pair and state for every submit (a state is spent once),
    // on the callback the hosted sign-in uses. Without one the registration
    // still goes through, and the athlete signs in as before.
    let prepared: RegistrationSignIn | null = null;
    try {
      prepared = await authApi.prepareRegistrationSignIn(
        Linking.createURL(SIGN_IN_CALLBACK_PATH),
        expoPkceCrypto,
      );
    } catch (error) {
      console.warn('Could not prepare the sign-in a registration carries:', error);
    }

    // A signup code, when the athlete has one, rides along for the server to check.
    const response = await authApi.register({
      email,
      password,
      display_name: displayName,
      turnstile_token: turnstileToken,
      ...(signupCode ? { signup_code: signupCode } : {}),
      ...(prepared ? { sign_in: prepared.request } : {}),
    });

    if (!prepared || response.activation !== 'active' || !response.sign_in_code) {
      return { response, signedIn: false };
    }
    // The account exists from here on: a code that cannot be redeemed sends
    // the athlete to sign in, never back to a "registration failed" whose
    // retry would answer that the address is taken.
    try {
      const session = await redeemCode({
        code: response.sign_in_code,
        codeVerifier: prepared.codeVerifier,
        redirectUri: prepared.redirectUri,
      });
      await adoptSession(session);
      return { response, signedIn: true };
    } catch (error) {
      // The status only: an HTTP error carries the request body, whose code,
      // PKCE verifier and device evidence must not reach a log.
      const status = (error as { response?: { status?: unknown } } | null)?.response?.status;
      console.warn('Could not redeem the sign-in code a registration returned, status:', status ?? 'none');
      return { response, signedIn: false };
    }
  }, [adoptSession]);

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
    signIn,
    loginWithFirebase,
    logout,
    register,
    updateUser,
  }), [user, isAuthenticated, isLoading, signIn, loginWithFirebase, logout, register, updateUser]);

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
