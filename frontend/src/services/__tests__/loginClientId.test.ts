// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the client_id each first-party app names on the password grant
// ABOUTME: The server refuses a password grant naming no first-party client (carnet#768)

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

/** Log in on `platform` and return the form the token endpoint received. */
async function loginForm(platform: 'web' | 'mobile'): Promise<URLSearchParams> {
  const post = vi.fn().mockResolvedValue({ data: {} })
  const axios = { post } as unknown as AxiosInstance
  await createAuthApi(axios, memoryStorage(), platform).login({
    email: 'athlete@example.com',
    password: 'password123',
  })
  expect(post).toHaveBeenCalledTimes(1)
  return new URLSearchParams(post.mock.calls[0][1] as string)
}

describe('password grant client_id', () => {
  it('names the web app on the web', async () => {
    const form = await loginForm('web')
    expect(form.get('grant_type')).toBe('password')
    expect(form.get('client_id')).toBe('dravr-web')
    expect(form.get('scope')).toBeNull()
  })

  it('names the mobile app on the phone, which also asks for a refresh token', async () => {
    const form = await loginForm('mobile')
    expect(form.get('client_id')).toBe('dravr-mobile')
    expect(form.get('scope')).toBe('offline_access')
  })
})
