// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest'

const { mockAxios } = vi.hoisted(() => ({
  mockAxios: {
    get: vi.fn(),
    post: vi.fn(),
    put: vi.fn(),
    delete: vi.fn(),
  },
}))

vi.mock('../api/client', () => ({
  axios: mockAxios,
}))

import { authApi } from '../api/auth'

describe('authApi', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  describe('login', () => {
    it('should send OAuth2 ROPC request with form-encoded body', async () => {
      const mockResponse = {
        data: { user: { id: '1', email: 'test@example.com' }, csrf_token: 'csrf-123' },
      }
      mockAxios.post.mockResolvedValue(mockResponse)

      const result = await authApi.login('test@example.com', 'password123')

      expect(mockAxios.post).toHaveBeenCalledWith(
        '/oauth/token',
        expect.any(URLSearchParams),
        { headers: { 'Content-Type': 'application/x-www-form-urlencoded' } }
      )

      // Verify the URLSearchParams content
      const params = mockAxios.post.mock.calls[0][1] as URLSearchParams
      expect(params.get('grant_type')).toBe('password')
      expect(params.get('username')).toBe('test@example.com')
      expect(params.get('password')).toBe('password123')

      expect(result).toEqual(mockResponse.data)
    })
  })

  describe('loginWithFirebase', () => {
    it('should send Firebase ID token', async () => {
      const mockResponse = { data: { user: { id: '1' } } }
      mockAxios.post.mockResolvedValue(mockResponse)

      const result = await authApi.loginWithFirebase('firebase-id-token')

      expect(mockAxios.post).toHaveBeenCalledWith('/api/auth/firebase', {
        id_token: 'firebase-id-token',
      })
      expect(result).toEqual(mockResponse.data)
    })
  })

  describe('logout', () => {
    it('should call logout endpoint', async () => {
      mockAxios.post.mockResolvedValue({})

      await authApi.logout()

      expect(mockAxios.post).toHaveBeenCalledWith('/api/auth/logout')
    })

    it('should not throw when logout API fails', async () => {
      const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {})
      mockAxios.post.mockRejectedValue(new Error('Network error'))

      await expect(authApi.logout()).resolves.toBeUndefined()

      expect(consoleError).toHaveBeenCalledWith('Logout API call failed:', expect.any(Error))
      consoleError.mockRestore()
    })
  })

  describe('register', () => {
    it('should send registration data', async () => {
      const mockResponse = { data: { user: { id: '1', email: 'new@example.com' } } }
      mockAxios.post.mockResolvedValue(mockResponse)

      const result = await authApi.register('new@example.com', 'password123', 'New User')

      expect(mockAxios.post).toHaveBeenCalledWith('/api/auth/register', {
        email: 'new@example.com',
        password: 'password123',
        display_name: 'New User',
      })
      expect(result).toEqual(mockResponse.data)
    })

    it('should send registration without display name', async () => {
      const mockResponse = { data: { user: { id: '1' } } }
      mockAxios.post.mockResolvedValue(mockResponse)

      await authApi.register('new@example.com', 'password123')

      expect(mockAxios.post).toHaveBeenCalledWith('/api/auth/register', {
        email: 'new@example.com',
        password: 'password123',
        display_name: undefined,
      })
    })
  })

  describe('refreshToken', () => {
    it('should call refresh endpoint', async () => {
      const mockResponse = { data: { access_token: 'new-token' } }
      mockAxios.post.mockResolvedValue(mockResponse)

      const result = await authApi.refreshToken()

      expect(mockAxios.post).toHaveBeenCalledWith('/api/auth/refresh')
      expect(result).toEqual(mockResponse.data)
    })
  })
})
