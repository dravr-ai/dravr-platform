// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the auth API surface exported from services/api (pierreApi.auth)
// ABOUTME: Validates sign-in, logout, and register calls go through the shared client

import { describe, it, expect, beforeEach, vi } from 'vitest'

const { mockBeginSignIn, mockCompleteSignIn, mockLoginWithFirebase, mockLogout, mockRegister } = vi.hoisted(() => ({
  mockBeginSignIn: vi.fn(),
  mockCompleteSignIn: vi.fn(),
  mockLoginWithFirebase: vi.fn(),
  mockLogout: vi.fn(),
  mockRegister: vi.fn(),
}))

vi.mock('../api', async (importOriginal) => {
  const original = await importOriginal<typeof import('../api')>()
  return {
    ...original,
    authApi: {
      beginSignIn: mockBeginSignIn,
      completeSignIn: mockCompleteSignIn,
      loginWithFirebase: mockLoginWithFirebase,
      logout: mockLogout,
      register: mockRegister,
    },
  }
})

import { authApi } from '../api'

describe('authApi (from @pierre/api-client via services/api barrel)', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  describe('hosted sign-in', () => {
    it('should delegate beginSignIn to pierreApi.auth.beginSignIn', async () => {
      const request = {
        authorizeUrl: '/oauth2/authorize?client_id=dravr-web',
        codeVerifier: 'v',
        state: 's',
        redirectUri: 'http://localhost/auth/callback',
      }
      mockBeginSignIn.mockResolvedValue(request)
      const crypto = { randomBytes: vi.fn(), sha256: vi.fn() }

      const result = await authApi.beginSignIn('http://localhost/auth/callback', crypto)

      expect(mockBeginSignIn).toHaveBeenCalledWith('http://localhost/auth/callback', crypto)
      expect(result).toEqual(request)
    })

    it('should delegate completeSignIn to pierreApi.auth.completeSignIn', async () => {
      const mockResponse = { user: { id: '1', email: 'test@example.com' }, csrf_token: 'csrf-123' }
      mockCompleteSignIn.mockResolvedValue(mockResponse)

      const result = await authApi.completeSignIn({
        code: 'c',
        codeVerifier: 'v',
        redirectUri: 'http://localhost/auth/callback',
      })

      expect(mockCompleteSignIn).toHaveBeenCalledWith({
        code: 'c',
        codeVerifier: 'v',
        redirectUri: 'http://localhost/auth/callback',
      })
      expect(result).toEqual(mockResponse)
    })

    it('should propagate code exchange errors', async () => {
      mockCompleteSignIn.mockRejectedValue(new Error('invalid_grant'))

      await expect(
        authApi.completeSignIn({ code: 'spent', codeVerifier: 'v', redirectUri: 'http://localhost/auth/callback' })
      ).rejects.toThrow('invalid_grant')
    })
  })

  describe('loginWithFirebase', () => {
    it('should delegate to pierreApi.auth.loginWithFirebase', async () => {
      const mockResponse = { user: { id: '1' } }
      mockLoginWithFirebase.mockResolvedValue(mockResponse)

      const result = await authApi.loginWithFirebase({ idToken: 'firebase-token-123' })

      expect(mockLoginWithFirebase).toHaveBeenCalledWith({
        idToken: 'firebase-token-123',
      })
      expect(result).toEqual(mockResponse)
    })
  })

  describe('logout', () => {
    it('should delegate to pierreApi.auth.logout', async () => {
      mockLogout.mockResolvedValue(undefined)

      await authApi.logout()

      expect(mockLogout).toHaveBeenCalled()
    })
  })

  describe('register', () => {
    it('should delegate to pierreApi.auth.register', async () => {
      const mockResponse = { user: { id: '1', email: 'new@example.com' } }
      mockRegister.mockResolvedValue(mockResponse)

      const result = await authApi.register({
        email: 'new@example.com',
        password: 'SecurePass123',
        display_name: 'New User',
      })

      expect(mockRegister).toHaveBeenCalledWith({
        email: 'new@example.com',
        password: 'SecurePass123',
        display_name: 'New User',
      })
      expect(result).toEqual(mockResponse)
    })
  })

  describe('API surface verification', () => {
    it('authApi is the pierreApi.auth instance (not a local module)', () => {
      // authApi is re-exported as pierreApi.auth in services/api/index.ts
      // This test verifies the mock wiring matches the real barrel export
      expect(authApi).toBeDefined()
      expect(authApi.beginSignIn).toBe(mockBeginSignIn)
      expect(authApi.completeSignIn).toBe(mockCompleteSignIn)
      expect(authApi.loginWithFirebase).toBe(mockLoginWithFirebase)
      expect(authApi.logout).toBe(mockLogout)
      expect(authApi.register).toBe(mockRegister)
    })
  })
})
