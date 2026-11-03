// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the bodies a sign-up sends with a landing campaign: utm_* beside the register fields and beside the Firebase id_token
// ABOUTME: Runs the real auth domain over a stub axios, so web and mobile share what is pinned here

import { describe, expect, it, vi } from 'vitest';
import type { AxiosInstance } from 'axios';
import { createAuthApi } from '../src/domains/auth';
import type { AuthStorage } from '../src/types/platform';

function stubAuthApi() {
  const post = vi.fn().mockResolvedValue({ data: {} });
  const authStorage = {
    setToken: vi.fn(),
    setCsrfToken: vi.fn(),
    setUser: vi.fn(),
  } as unknown as AuthStorage;
  return { post, api: createAuthApi({ post } as unknown as AxiosInstance, authStorage, 'web') };
}

describe('auth domain — signup attribution', () => {
  it('posts the campaign beside the Firebase id_token', async () => {
    const { post, api } = stubAuthApi();

    await api.loginWithFirebase({
      idToken: 'firebase-id',
      attribution: { utm_source: 'newsletter', utm_medium: 'email', utm_campaign: 'fall' },
    });

    expect(post).toHaveBeenCalledWith('/api/auth/firebase', {
      id_token: 'firebase-id',
      utm_source: 'newsletter',
      utm_medium: 'email',
      utm_campaign: 'fall',
    });
  });

  it('posts only the id_token when there is no campaign', async () => {
    const { post, api } = stubAuthApi();

    await api.loginWithFirebase({ idToken: 'firebase-id' });

    expect(post.mock.calls[0][1]).toStrictEqual({ id_token: 'firebase-id' });
  });

  it('posts the campaign in the register body', async () => {
    const { post, api } = stubAuthApi();

    await api.register({ email: 'new@example.com', password: 'password123', utm_source: 'strava-club' });

    expect(post).toHaveBeenCalledWith('/api/auth/register', {
      email: 'new@example.com',
      password: 'password123',
      utm_source: 'strava-club',
    });
  });
});
