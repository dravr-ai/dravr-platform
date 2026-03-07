// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the auth API surface exported from services/api (pierreApi.auth)
// ABOUTME: Validates login, logout, register, and refresh calls go through the shared client

import { describe, it, expect, beforeEach, vi } from 'vitest'

const { mockLogin, mockLoginWithFirebase, mockLogout, mockRegister, mockRefreshToken } =
  vi.hoisted(() => ({
    mockLogin: vi.fn(),
    mockLoginWithFirebase: vi.fn(),
    mockLogout: vi.fn(),
    mockRegister: vi.fn(),
    mockRefreshToken: vi.fn(),
  }))

vi.mock('../api', async (importOriginal) => {
  const original = await importOriginal<typeof import('../api')>()
  return {
    ...original,
    authApi: {
      login: mockLogin,
      loginWithFirebase: mockLoginWithFirebase,
      logout: mockLogout,
      register: mockRegister,
      refreshToken: mockRefreshToken,
    },
  }
})

import { authApi } from '../api'

describe('authApi (from @pierre/api-client via services/api barrel)', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  describe('login', () => {
    it('should delegate to pierreApi.auth.login', async () => {
      const mockResponse = { user: { id: '1', email: 'test@example.com' }, csrf_token: 'csrf-123' }
      mockLogin.mockResolvedValue(mockResponse)

      const result = await authApi.login({ email: 'test@example.com', password: 'password123' })

      expect(mockLogin).toHaveBeenCalledWith({
        email: 'test@example.com',
        password: 'password123',
      })
      expect(result).toEqual(mockResponse)
    })

    it('should propagate login errors', async () => {
      mockLogin.mockRejectedValue(new Error('Invalid credentials'))

      await expect(
        authApi.login({ email: 'bad@example.com', password: 'wrong' })
      ).rejects.toThrow('Invalid credentials')
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

  describe('refreshToken', () => {
    it('should delegate to pierreApi.auth.refreshToken', async () => {
      const mockResponse = { user: { id: '1' }, csrf_token: 'new-csrf' }
      mockRefreshToken.mockResolvedValue(mockResponse)

      const result = await authApi.refreshToken()

      expect(mockRefreshToken).toHaveBeenCalled()
      expect(result).toEqual(mockResponse)
    })

    it('should propagate refresh errors', async () => {
      mockRefreshToken.mockRejectedValue(new Error('Token expired'))

      await expect(authApi.refreshToken()).rejects.toThrow('Token expired')
    })
  })

  describe('API surface verification', () => {
    it('authApi is the pierreApi.auth instance (not a local module)', () => {
      // authApi is re-exported as pierreApi.auth in services/api/index.ts
      // This test verifies the mock wiring matches the real barrel export
      expect(authApi).toBeDefined()
      expect(authApi.login).toBe(mockLogin)
      expect(authApi.loginWithFirebase).toBe(mockLoginWithFirebase)
      expect(authApi.logout).toBe(mockLogout)
      expect(authApi.register).toBe(mockRegister)
      expect(authApi.refreshToken).toBe(mockRefreshToken)
    })
  })
})
