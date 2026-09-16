// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Authentication domain API - login, logout, register, session restore
// ABOUTME: Platform-agnostic auth logic using the adapter for token storage

import type { AxiosInstance } from 'axios';
import type { User, LoginResponse, RegisterResponse, FirebaseLoginResponse, SessionResponse } from '@pierre/shared-types';
import type { AuthStorage, PlatformAdapter } from '../types/platform';
import { ENDPOINTS } from '../core/endpoints';

/**
 * The scope a login sends to receive a refresh token alongside its JWT.
 *
 * OpenID Connect's name for "I will need to act while the user is away".
 * Only the mobile app asks: it keeps the token in the device keychain and
 * exchanges it when the JWT lapses. The web app's session is the httpOnly
 * cookie, and a token it would discard is a live credential in a table.
 */
const OFFLINE_ACCESS_SCOPE = 'offline_access';

export interface LoginCredentials {
  email: string;
  password: string;
}

export interface RegisterCredentials {
  email: string;
  password: string;
  display_name?: string;
}

export interface FirebaseLoginData {
  idToken: string;
}

/**
 * Creates the auth API methods bound to an axios instance.
 *
 * `platform` decides whether a login asks for a refresh token: the phone
 * can hold one, the browser has its cookie.
 */
export function createAuthApi(
  axios: AxiosInstance,
  authStorage: AuthStorage,
  platform: PlatformAdapter['platform']
) {
  return {
    /**
     * Login with email and password.
     */
    async login(credentials: LoginCredentials): Promise<LoginResponse> {
      const formData = new URLSearchParams();
      formData.append('grant_type', 'password');
      formData.append('username', credentials.email);
      formData.append('password', credentials.password);
      if (platform === 'mobile') {
        formData.append('scope', OFFLINE_ACCESS_SCOPE);
      }

      const response = await axios.post<LoginResponse>(ENDPOINTS.AUTH.TOKEN, formData.toString(), {
        headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
      });

      const data = response.data;

      // Store tokens
      if (data.access_token) {
        await authStorage.setToken(data.access_token);
      }
      if (data.refresh_token) {
        await authStorage.setRefreshToken(data.refresh_token);
      }
      if (data.user) {
        await authStorage.setUser(data.user);
      }

      return data;
    },

    /**
     * Login with Firebase ID token.
     */
    async loginWithFirebase(data: FirebaseLoginData): Promise<FirebaseLoginResponse> {
      // Backend expects id_token (snake_case)
      const response = await axios.post<FirebaseLoginResponse>(ENDPOINTS.AUTH.FIREBASE, {
        id_token: data.idToken,
      });
      const result = response.data;

      // Store tokens - Firebase uses jwt_token, not access_token
      if (result.jwt_token) {
        await authStorage.setToken(result.jwt_token);
      }
      if (result.csrf_token) {
        await authStorage.setCsrfToken(result.csrf_token);
      }
      if (result.user) {
        await authStorage.setUser(result.user);
      }

      return result;
    },

    /**
     * Logout the current user.
     *
     * A device holding a refresh token hands it back so the server revokes
     * it; otherwise the token would stay exchangeable for a month after the
     * phone forgot it. The web app has none and sends an empty body.
     */
    async logout(): Promise<void> {
      try {
        const refreshToken = await authStorage.getRefreshToken();
        await axios.post(ENDPOINTS.AUTH.LOGOUT, refreshToken ? { refresh_token: refreshToken } : undefined);
      } finally {
        // Always clear local auth data
        await authStorage.clear();
      }
    },

    /**
     * Register a new user.
     */
    async register(credentials: RegisterCredentials): Promise<RegisterResponse> {
      const response = await axios.post<RegisterResponse>(ENDPOINTS.AUTH.REGISTER, credentials);
      return response.data;
    },

    /**
     * Restore session using httpOnly cookie authentication.
     * Returns user info and a fresh JWT for WebSocket auth.
     * Throws on 401 if no valid session exists.
     */
    async getSession(): Promise<SessionResponse> {
      const response = await axios.get<SessionResponse>(ENDPOINTS.AUTH.SESSION);
      return response.data;
    },

    /**
     * Get the currently stored user (from local storage, not server).
     */
    async getStoredUser(): Promise<User | null> {
      return authStorage.getUser<User>();
    },

    /**
     * Store user data locally.
     */
    async storeUser(user: User): Promise<void> {
      await authStorage.setUser(user);
    },

    /**
     * Store all auth data (token, CSRF token, and user).
     * Mobile-compatible convenience method.
     */
    async storeAuth(token: string, csrfToken: string, user: User): Promise<void> {
      await authStorage.setToken(token);
      await authStorage.setCsrfToken(csrfToken);
      await authStorage.setUser(user);
    },

    /**
     * Initialize auth state from storage.
     * Returns true if valid auth data was found.
     */
    async initializeAuth(): Promise<boolean> {
      const token = await authStorage.getToken();
      const user = await authStorage.getUser<User>();
      return !!(token && user);
    },

    /**
     * Request a password reset code to be emailed to the given email.
     * Always returns success to prevent account enumeration.
     */
    async forgotPassword(email: string): Promise<{ message: string }> {
      const response = await axios.post<{ message: string }>(
        ENDPOINTS.AUTH.FORGOT_PASSWORD,
        { email },
      );
      return response.data;
    },

    /**
     * Re-send the address-confirmation link.
     *
     * Always resolves with the same neutral message whether or not the address
     * exists, is already confirmed, or has tripped its hourly cap — the response
     * must not let a caller probe which addresses are registered.
     */
    async resendVerification(email: string): Promise<{ message: string }> {
      const response = await axios.post<{ message: string }>(
        ENDPOINTS.AUTH.RESEND_VERIFICATION,
        { email },
      );
      return response.data;
    },

    /**
     * Complete a password reset using the emailed reset code and new password.
     */
    async resetPassword(
      code: string,
      newPassword: string,
    ): Promise<{ message: string }> {
      const response = await axios.post<{ message: string }>(
        ENDPOINTS.AUTH.COMPLETE_RESET,
        { reset_token: code, new_password: newPassword },
      );
      return response.data;
    },
  };
}

export type AuthApi = ReturnType<typeof createAuthApi>;
