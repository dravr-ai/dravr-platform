// ABOUTME: Pins the sign-up guard calls of the shared auth client: the public config read and the token on register
// ABOUTME: Runs the real createAuthApi against a recording axios, so the wire path and body are what is asserted (carnet#1063)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi } from 'vitest'
import type { AxiosInstance } from 'axios'
import { createAuthApi } from '@pierre/api-client'
import type { AuthStorage } from '@pierre/api-client'

function memoryStorage(): AuthStorage {
  return {
    getToken: vi.fn().mockResolvedValue(null),
    setToken: vi.fn().mockResolvedValue(undefined),
    removeToken: vi.fn().mockResolvedValue(undefined),
    getCsrfToken: vi.fn().mockResolvedValue(null),
    setCsrfToken: vi.fn().mockResolvedValue(undefined),
    getUser: vi.fn().mockResolvedValue(null),
    setUser: vi.fn().mockResolvedValue(undefined),
    getRefreshToken: vi.fn().mockResolvedValue(null),
    setRefreshToken: vi.fn().mockResolvedValue(undefined),
    clear: vi.fn().mockResolvedValue(undefined),
  }
}

describe('sign-up guards in the shared auth client', () => {
  it('reads the public sign-up config from /api/auth/signup-config', async () => {
    const config = { open: false, turnstile_site_key: '1x00000000000000000000AA' }
    const get = vi.fn().mockResolvedValue({ data: config })
    const axios = { get, defaults: {} } as unknown as AxiosInstance

    const result = await createAuthApi(axios, memoryStorage(), 'web').getSignupConfig()

    expect(get).toHaveBeenCalledWith('/api/auth/signup-config')
    expect(result).toEqual(config)
  })

  it('sends the Turnstile token beside the credentials on register', async () => {
    const response = {
      user_id: 'u1',
      email: 'new@example.com',
      user_status: 'pending',
      message: 'Confirm your email to activate your account.',
    }
    const post = vi.fn().mockResolvedValue({ data: response })
    const axios = { post, defaults: {} } as unknown as AxiosInstance

    const result = await createAuthApi(axios, memoryStorage(), 'mobile').register({
      email: 'new@example.com',
      password: 'password123',
      display_name: 'New',
      turnstile_token: 'tok-1',
    })

    expect(post).toHaveBeenCalledWith('/api/auth/register', {
      email: 'new@example.com',
      password: 'password123',
      display_name: 'New',
      turnstile_token: 'tok-1',
    })
    expect(result.message).toBe('Confirm your email to activate your account.')
  })
})
