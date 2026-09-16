// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Trades the stored refresh token for a fresh JWT, once at a time, so a lapsed session renews instead of ending
// ABOUTME: Platform-agnostic — a storage with no refresh token (the web app) answers false and the caller signs out as before

import type { AxiosInstance } from 'axios';
import type { LoginResponse } from '@pierre/shared-types';
import type { AuthStorage } from '../types/platform';
import { ENDPOINTS } from '../core/endpoints';

/**
 * Attempt to renew the session from the stored refresh token.
 *
 * Resolves `true` when a new JWT is in storage and the failed request can be
 * retried, `false` when there is nothing to renew with or the server refused.
 */
export type SessionRefresher = () => Promise<boolean>;

/**
 * Build the refresher for one client.
 *
 * Single-flight: every 401 that lands while an exchange is in progress waits
 * on the same promise. Without that, a screen that fires four requests on a
 * cold start would race four exchanges of one token, and only the first can
 * win — the server revokes a token as it exchanges it, and treats the other
 * three as a replayed credential, killing the session they were trying to
 * save.
 */
export function createSessionRefresher(
  instance: AxiosInstance,
  authStorage: AuthStorage
): SessionRefresher {
  let inFlight: Promise<boolean> | null = null;

  return () => {
    if (!inFlight) {
      inFlight = exchangeRefreshToken(instance, authStorage).finally(() => {
        inFlight = null;
      });
    }
    return inFlight;
  };
}

/**
 * One exchange: `grant_type=refresh_token` against the token endpoint.
 *
 * The server rotates on every exchange — the response carries a successor and
 * the token just presented is dead — so the successor is stored before this
 * resolves. A refused exchange (expired, revoked, replayed) answers 400, not
 * 401, so it never re-enters the interceptor that called here.
 */
async function exchangeRefreshToken(
  instance: AxiosInstance,
  authStorage: AuthStorage
): Promise<boolean> {
  const refreshToken = await authStorage.getRefreshToken();
  if (!refreshToken) return false;

  const form = new URLSearchParams();
  form.append('grant_type', 'refresh_token');
  form.append('refresh_token', refreshToken);

  try {
    const response = await instance.post<LoginResponse>(ENDPOINTS.AUTH.TOKEN, form.toString(), {
      headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    });
    const data = response.data;
    if (!data.access_token) return false;

    await authStorage.setToken(data.access_token);
    if (data.refresh_token) {
      await authStorage.setRefreshToken(data.refresh_token);
    }
    if (data.csrf_token) {
      await authStorage.setCsrfToken(data.csrf_token);
    }
    if (data.user) {
      await authStorage.setUser(data.user);
    }
    return true;
  } catch {
    return false;
  }
}
