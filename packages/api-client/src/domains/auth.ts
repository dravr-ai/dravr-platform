// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Authentication domain API - first-party sign-in (authorization code + PKCE), logout, register, session restore
// ABOUTME: Platform-agnostic auth logic using the adapter for token storage

import type { AxiosInstance } from 'axios';
import type { User, LoginResponse, RegisterResponse, FirebaseLoginResponse, SessionResponse } from '@pierre/shared-types';
import type { AuthStorage, PlatformAdapter } from '../types/platform';
import { ENDPOINTS } from '../core/endpoints';
import { createPkcePair, type PkceCrypto } from '../core/pkce';

/**
 * The scope a login sends to receive a refresh token alongside its JWT.
 *
 * OpenID Connect's name for "I will need to act while the user is away".
 * Only the mobile app asks: it keeps the token in the device keychain and
 * exchanges it when the JWT lapses. The web app's session is the httpOnly
 * cookie, and a token it would discard is a live credential in a table.
 */
const OFFLINE_ACCESS_SCOPE = 'offline_access';

/**
 * The `client_id` each first-party app signs in as.
 *
 * Dravr's apps sign in like any OAuth client (carnet#787): the athlete types
 * the password on the server's hosted login page, never into the app, and
 * the app redeems the authorization code it is sent back with, proving with
 * its PKCE verifier that it started the sign-in. The identifiers are public,
 * as every browser and native app's is. Mirrors `WEB_CLIENT_ID` and
 * `MOBILE_CLIENT_ID` in `crates/pierre-auth/src/oauth2_server/first_party.rs`.
 */
const FIRST_PARTY_CLIENT_IDS = {
  web: 'dravr-web',
  mobile: 'dravr-mobile',
} as const satisfies Record<PlatformAdapter['platform'], string>;

/** A sign-in the app has started: where to send the athlete, and what to keep until they return. */
export interface SignInRequest {
  /** The hosted sign-in to open: `/oauth2/authorize` on the API's origin. */
  authorizeUrl: string;
  /** Kept by the app and sent with the code; never leaves the device before then. */
  codeVerifier: string;
  /** Must come back unchanged on the callback, or the callback is not this sign-in's. */
  state: string;
  /** Where the server sends the athlete back; sent again with the code. */
  redirectUri: string;
}

/** What the server sent the athlete back with: a code to redeem, or the reason there is none. */
export type SignInCallback =
  | { kind: 'code'; code: string }
  | { kind: 'error'; error: string; description?: string };

/**
 * Read the callback the hosted sign-in returned to.
 *
 * `params` are the callback URL's query parameters. A `state` that is not
 * the one this sign-in sent is refused: the callback was not started here.
 */
export function readSignInCallback(params: URLSearchParams, expectedState: string): SignInCallback {
  const error = params.get('error');
  if (error) {
    return { kind: 'error', error, description: params.get('error_description') ?? undefined };
  }
  if (params.get('state') !== expectedState) {
    return { kind: 'error', error: 'state_mismatch' };
  }
  const code = params.get('code');
  if (!code) {
    return { kind: 'error', error: 'invalid_request' };
  }
  return { kind: 'code', code };
}

/**
 * The iOS app's App Attest evidence for a code exchange (carnet#810): the
 * key's attestation on an install's first sign-in, an assertion by it on
 * every later one, both signed over the authorization code.
 */
export type AppAttestEvidence =
  | { keyId: string; attestation: string }
  | { keyId: string; assertion: string };

export interface CompleteSignIn {
  code: string;
  codeVerifier: string;
  redirectUri: string;
  appAttest?: AppAttestEvidence;
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
     * Start a sign-in: a fresh PKCE pair and `state`, and the hosted sign-in
     * URL to open with them, in `uiLocale` when given. The app keeps the
     * result until the athlete returns to `redirectUri`.
     */
    async beginSignIn(redirectUri: string, crypto: PkceCrypto, uiLocale?: string): Promise<SignInRequest> {
      const { codeVerifier, codeChallenge, state } = await createPkcePair(crypto);
      const query = new URLSearchParams({
        response_type: 'code',
        client_id: FIRST_PARTY_CLIENT_IDS[platform],
        redirect_uri: redirectUri,
        code_challenge: codeChallenge,
        code_challenge_method: 'S256',
        state,
        // Always ask for the password: the browser may still hold the
        // session of whoever signed in before, and signing out of the app
        // must not leave the next sign-in landing in their account.
        prompt: 'login',
      });
      // OpenID Connect ui_locales: the hosted page speaks the language the
      // app is showing, not whatever the browser or OS happens to prefer.
      if (uiLocale) {
        query.set('ui_locales', uiLocale);
      }
      const origin = (axios.defaults.baseURL ?? '').replace(/\/+$/, '');
      return {
        authorizeUrl: `${origin}${ENDPOINTS.AUTH.AUTHORIZE}?${query.toString()}`,
        codeVerifier,
        state,
        redirectUri,
      };
    },

    /**
     * Redeem the code the hosted sign-in returned with for a session: the
     * same response, cookies and stored tokens a sign-in has always produced.
     */
    async completeSignIn(request: CompleteSignIn): Promise<LoginResponse> {
      const formData = new URLSearchParams();
      formData.append('grant_type', 'authorization_code');
      formData.append('client_id', FIRST_PARTY_CLIENT_IDS[platform]);
      formData.append('code', request.code);
      formData.append('redirect_uri', request.redirectUri);
      formData.append('code_verifier', request.codeVerifier);
      if (platform === 'mobile') {
        formData.append('scope', OFFLINE_ACCESS_SCOPE);
      }
      if (request.appAttest) {
        formData.append('app_attest_key_id', request.appAttest.keyId);
        if ('attestation' in request.appAttest) {
          formData.append('app_attest_attestation', request.appAttest.attestation);
        } else {
          formData.append('app_attest_assertion', request.appAttest.assertion);
        }
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
