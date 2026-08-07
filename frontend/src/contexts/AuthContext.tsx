// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState, useEffect, useCallback, useRef } from 'react';
import { authApi, adminApi, pierreApi, userApi } from '../services/api';
import { AuthContext } from './auth';
import type { User, ImpersonationState } from './auth';

const STORAGE_KEYS = {
  USER: 'pierre_user',
  IMPERSONATION: 'pierre_impersonation',
} as const;

const defaultImpersonationState: ImpersonationState = {
  isImpersonating: false,
  targetUser: null,
  sessionId: null,
  originalUser: null,
};

/**
 * Read the browser's IANA timezone via `Intl.DateTimeFormat` and PUT it
 * to the server so the chat prompt can resolve `{{CURRENT_DATE}}`
 * locally. Best-effort: failures are logged to the console but never
 * surfaced as errors, since the date anchor's UTC fallback keeps chat
 * working when the timezone capture path is unavailable.
 *
 * Only runs for active users. Pending/suspended accounts are gated out of
 * feature endpoints by the account-status policy, so PUT /users/me/timezone
 * returns 401 for them — which the api-client's global 401 interceptor turns
 * into a forced logout, bouncing the user off the "pending approval" screen
 * straight back to login. Timezone only matters for chat (active users), so
 * skip it for everyone else.
 */
async function captureUserTimezone(userStatus: string | undefined): Promise<void> {
  if (userStatus !== 'active') {
    return;
  }
  try {
    const tz = Intl.DateTimeFormat().resolvedOptions().timeZone;
    if (tz) {
      await userApi.setTimezone(tz);
    }
  } catch (err) {
    // Non-fatal: log and continue. The next login attempt will retry.
    // eslint-disable-next-line no-console
    console.warn('Failed to persist user timezone:', err);
  }
}

export function AuthProvider({ children }: { children: React.ReactNode }) {
  const [user, setUser] = useState<User | null>(null);
  const [token, setToken] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [impersonation, setImpersonation] = useState<ImpersonationState>(defaultImpersonationState);

  // Guard against re-entrant logout calls (prevents infinite loop when
  // the logout POST itself returns 401 and fires another auth failure event)
  const isLoggingOutRef = useRef(false);

  useEffect(() => {
    // Show cached user immediately for instant UI render
    const storedUser = localStorage.getItem(STORAGE_KEYS.USER);
    const storedImpersonation = localStorage.getItem(STORAGE_KEYS.IMPERSONATION);

    if (storedUser && storedUser !== 'undefined') {
      setUser(JSON.parse(storedUser));
    }
    if (storedImpersonation) {
      setImpersonation(JSON.parse(storedImpersonation));
    }

    // Restore session from httpOnly cookie (gets fresh JWT for WebSocket)
    if (storedUser) {
      authApi.getSession()
        .then((session) => {
          setUser(session.user);
          setToken(session.access_token);
          pierreApi.adapter.authStorage.setCsrfToken(session.csrf_token);
          localStorage.setItem(STORAGE_KEYS.USER, JSON.stringify(session.user));
        })
        .catch(() => {
          // Cookie expired or invalid — clear cached user
          setUser(null);
          setToken(null);
          localStorage.removeItem(STORAGE_KEYS.USER);
        })
        .finally(() => {
          setIsLoading(false);
        });
    } else {
      setIsLoading(false);
    }

    // Listen for auth failures from pierreApi's 401 interceptor
    const handleAuthFailure = () => {
      // Skip if already logging out or already on login page
      if (isLoggingOutRef.current) return;
      if (window.location.pathname.includes('/login')) return;
      logout();
    };

    window.addEventListener('pierre:auth:failure', handleAuthFailure);

    return () => {
      window.removeEventListener('pierre:auth:failure', handleAuthFailure);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const login = async (email: string, password: string) => {
    const response = await authApi.login({ email, password });
    // Re-arm the logout guard for this new session. logout() latches it and
    // never releases it, so that a burst of 401s from a dead session collapses
    // into a single logout; a successful login is what makes a future logout
    // meaningful again.
    isLoggingOutRef.current = false;
    // OAuth2 ROPC response uses access_token, csrf_token, and user
    const { access_token, csrf_token, user: userData } = response;

    // Store CSRF token via pierreApi's auth storage (writes to localStorage)
    await pierreApi.adapter.authStorage.setCsrfToken(csrf_token);

    // Keep JWT in React state only (for WebSocket auth) — httpOnly cookie handles REST
    if (access_token) {
      setToken(access_token);
    }

    // Store user info in state and localStorage (for instant UI render on refresh)
    setUser(userData);
    localStorage.setItem(STORAGE_KEYS.USER, JSON.stringify(userData));

    // Best-effort: forward the browser's IANA timezone so the chat
    // prompt can render {{CURRENT_DATE}} in the user's local calendar
    // day (otherwise the resolver falls back to UTC and the coach
    // misreads "today"). A failure here must not break login — swallow
    // it and log to the browser console.
    void captureUserTimezone(userData.user_status);
  };

  const loginWithFirebase = async (idToken: string) => {
    const response = await authApi.loginWithFirebase({ idToken });
    const { csrf_token, jwt_token, user: userData } = response;

    // Store CSRF token via pierreApi's auth storage (writes to localStorage)
    await pierreApi.adapter.authStorage.setCsrfToken(csrf_token);

    // Keep JWT in React state only (for WebSocket auth) — httpOnly cookie handles REST
    if (jwt_token) {
      setToken(jwt_token);
    }

    // Store user info in state and localStorage (for instant UI render on refresh)
    setUser(userData);
    localStorage.setItem(STORAGE_KEYS.USER, JSON.stringify(userData));

    // Best-effort: capture the browser's IANA timezone (see notes in
    // the password-login branch above).
    void captureUserTimezone(userData.user_status);

    return response;
  };

  const logout = useCallback(() => {
    // Prevent re-entrant calls (logout POST returning 401 would trigger this again)
    if (isLoggingOutRef.current) return;
    isLoggingOutRef.current = true;

    // If impersonating, also clear impersonation state
    if (impersonation.isImpersonating) {
      setImpersonation(defaultImpersonationState);
      localStorage.removeItem(STORAGE_KEYS.IMPERSONATION);
    }

    setUser(null);
    setToken(null);

    // Send logout request first (while CSRF token is still available),
    // then clear local storage. authApi.logout() also clears authStorage
    // in its finally block, so we only need to remove the user key here.
    authApi.logout()
      .catch((error) => {
        console.error('Logout API call failed:', error);
        // Continue with local cleanup even if API fails
      })
      .finally(() => {
        localStorage.removeItem(STORAGE_KEYS.USER);
        // Deliberately NOT clearing isLoggingOutRef here. Releasing it when the
        // POST settles only guards against CONCURRENT re-entry, while the 401s
        // that trigger logout arrive sequentially — each later one found the
        // flag clear and fired another full logout (5 observed against a dead
        // session). Once logout has begun, further auth failures are redundant;
        // login() re-arms the flag for the next session.
      });
  }, [impersonation.isImpersonating]);

  const startImpersonation = useCallback(async (targetUserId: string, reason?: string) => {
    if (!user || user.role !== 'super_admin') {
      throw new Error('Only super admins can impersonate users');
    }

    const response = await adminApi.startImpersonation(targetUserId, reason);

    // Store original user before switching
    const newImpersonationState: ImpersonationState = {
      isImpersonating: true,
      targetUser: response.target_user,
      sessionId: response.session_id,
      originalUser: user,
    };

    setImpersonation(newImpersonationState);
    localStorage.setItem(STORAGE_KEYS.IMPERSONATION, JSON.stringify(newImpersonationState));

    // Update token to impersonation token (React state only)
    setToken(response.token);
  }, [user]);

  const endImpersonation = useCallback(async () => {
    if (!impersonation.isImpersonating) {
      return;
    }

    try {
      await adminApi.endImpersonation();
    } catch (error) {
      console.error('Failed to end impersonation on server:', error);
      // Continue with local cleanup even if API fails
    }

    // Restore original user
    if (impersonation.originalUser) {
      setUser(impersonation.originalUser);
      localStorage.setItem(STORAGE_KEYS.USER, JSON.stringify(impersonation.originalUser));
    }

    // Clear impersonation state
    setImpersonation(defaultImpersonationState);
    localStorage.removeItem(STORAGE_KEYS.IMPERSONATION);

    // Re-login to get fresh tokens for the original user
    // The user will need to log in again for simplicity
    // In a more sophisticated implementation, we could store the original token
    window.location.reload();
  }, [impersonation]);

  const value = {
    user,
    token,
    isAuthenticated: !!user,
    isLoading,
    loading: isLoading, // For test compatibility
    login,
    loginWithFirebase,
    logout,
    impersonation,
    startImpersonation,
    endImpersonation,
  };

  return (
    <AuthContext.Provider value={value}>
      {children}
    </AuthContext.Provider>
  );
}
